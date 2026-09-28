#!/usr/bin/env node
// TSK MCP server —— 把 TSK 的沙箱分析/检索暴露成 Claude Code 的 MCP 工具。
// 机制移植自 context-mode：工具响应由本 server 掌控（只回蒸馏/检索结果），原文不进模型 → 保证 per-tool 压缩比。
//
// 工具：
//   tsk_analyze(file, script?)  —— 沙箱分析大文件，只回摘要/计算结果（node-stdout）
//   tsk_search(query, top?)     —— 检索 TSK 已外置/知识库全文，回精确命中片段
//   tsk_exec(code, input?)      —— 沙箱跑一段处理脚本，只回 stdout
//
// 依赖：tsk 二进制在 PATH。stdio 传输（JSON-RPC 2.0，MCP Content-Length 帧 + 兼容逐行 JSON）。
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const W = "tsk MCP";
const TSK = process.env.TSK_BIN || findTsk();
const MAX_LOG = 200000; // 单个工具输出上限（防御）

function findTsk() {
  for (const d of [path.join(os.homedir(), "bin"), path.join(os.homedir(), ".local", "bin")]) {
    const cand = path.join(d, process.platform === "win32" ? "tsk.exe" : "tsk");
    if (fs.existsSync(cand)) return cand;
  }
  return "tsk";
}
const runTsk = (args, input) => {
  try {
    const r = spawnSync(TSK, args, { input: input ?? "", encoding: "utf8", timeout: 30000, maxBuffer: MAX_LOG + 1 });
    return { ok: r.status === 0, stdout: r.stdout ?? "", stderr: r.stderr ?? "" };
  } catch (e) {
    return { ok: false, stdout: "", stderr: String(e && e.message || e) };
  }
};

// —— 工具实现：只回「计算结果」，不带原文 ——
const tools = {
  tsk_analyze: {
    description: "在 TSK 沙箱里分析一个大文件，只把计算/摘要结果放回上下文（原文留在磁盘）。" +
      "适合日志/测试输出/CSV/构建输出/git-log 的『汇总统计 / 错误列表 / 结论』。对 100 行以上大文件，优先于 Read 读全文。参数 file 为路径，script 可选分析脚本路径。无损。",
    inputSchema: { type: "object", properties: { file: { type: "string" }, script: { type: "string" } }, required: ["file"] },
    call: async (a) => runTsk(["analyze", String(a.file)].concat(a.script ? ["--script", String(a.script)] : [])),
  },
  tsk_search: {
    description: "在 TSK 已索引/外置的全文里检索关键词，返回精确命中片段（代码块原样保留）。" +
      "适合『需要某段精确原文/API 签名/报错行』时按需取回，而不是整读大文件。参数 query 为查询串，top 最多返回数。",
    inputSchema: { type: "object", properties: { query: { type: "string" }, top: { type: "number" } }, additionalProperties: false },
    call: async (a) => runTsk(["search", String(a.query), "--top", String(a.top ?? 5)]),
  },
  tsk_exec: {
    description: "在 TSK 沙箱里跑一段处理代码，只回 stdout（计算结果）。适合对大数据做自定义聚合/分析，替代整读。code 为要执行的脚本路径。",
    inputSchema: { type: "object", properties: { code: { type: "string" }, input: { type: "string" } }, additionalProperties: false },
    call: async (a) => runTsk(["exec", String(a.code), String(a.input ?? "")]),
  },
};

// —— MCP stdio：默认逐行 JSON（MCP 标准），兼容 Content-Length 帧 ——
let buf = Buffer.alloc(0);
const pending = [];
function peekHeaderEnd(s) {
  const a = s.indexOf("\r\n\r\n"), b = s.indexOf("\n\n");
  if (a < 0 && b < 0) return -1;
  if (a < 0) return b;
  if (b < 0) return a;
  return a < b ? a : b;
}
function onData(chunk) {
  buf = Buffer.concat([buf, chunk]);
  for (;;) {
    const s = buf.toString("utf8");
    const he = peekHeaderEnd(s);
    if (he >= 0 && /^Content-Length:\s*\d+/i.test(s.slice(0, he))) {
      const m = /Content-Length:\s*(\d+)/i.exec(s);
      const bodyStart = he + (s.slice(he, he + 4) === "\r\n\r\n" ? 4 : 2);
      const len = parseInt(m[1], 10);
      if (buf.length < bodyStart + len) return;
      const body = buf.slice(bodyStart, bodyStart + len).toString("utf8");
      buf = buf.slice(bodyStart + len);
      try { pending.push(JSON.parse(body)); } catch {}
      continue;
    }
    // 逐行 JSON
    const nl = buf.indexOf("\n");
    if (nl < 0) return;
    const line = buf.slice(0, nl).toString("utf8").replace(/\r$/, "");
    buf = buf.slice(nl + 1);
    if (!line.trim()) continue;
    try { pending.push(JSON.parse(line)); } catch {}
  }
}
process.stdin.on("data", onData);
let ended = false;
process.stdin.on("end", () => { ended = true; });
process.stdin.on("close", () => { ended = true; });
let wake = null;
function nextMessage() {
  if (pending.length) return Promise.resolve(pending.shift());
  if (ended) return Promise.resolve(null);
  return new Promise((resolve) => {
    wake = () => { wake = null; resolve(pending.length ? pending.shift() : null); };
  });
}
// pending 入队后唤醒等待者
const origPush = pending.push;
pending.push = (v) => { const r = origPush.call(pending, v); if (wake) wake(); return r; };
const send = (obj) => process.stdout.write(JSON.stringify(obj) + "\n");

async function main() {
  // handshake
  const init = await nextMessage();
  if (init && init.method === "initialize") {
    send(makeResponse(init.id, {
      protocolVersion: "2024-11-05",
      capabilities: { tools: {} },
      serverInfo: { name: "tsk", version: "0.1.0" },
    }));
  }
  for (;;) {
    const msg = await nextMessage();
    if (!msg) break;
    if (msg.method === "notifications/initialized" || msg.method === "ping") continue;
    if (msg.method === "tools/list") { send(makeResponse(msg.id, { tools: Object.entries(tools).map(([n, t]) => ({ name: n, description: t.description, inputSchema: t.inputSchema })) })); continue; }
    if (msg.method === "tools/call") {
      const { name, arguments: args } = msg.params || {};
      const t = tools[name];
      if (!t) { send(makeResponse(msg.id, { content: [{ type: "text", text: `unknown tool ${name}` }], isError: true })); continue; }
      try {
        const r = await t.call(args || {});
        const text = r.ok
          ? (r.stdout || "(no output)").slice(0, MAX_LOG)
          : `[tsk error] ${(r.stderr || r.stdout || "failed").slice(0, 2000)}`;
        send(makeResponse(msg.id, { content: [{ type: "text", text }] }));
      } catch (e) {
        send(makeResponse(msg.id, { content: [{ type: "text", text: `[tsk mcp error] ${e && e.message || e}` }], isError: true }));
      }
      continue;
    }
    // unknown request: respond method-not-found? just ignore or respond
    if (msg.id !== undefined) send(makeResponse(msg.id, { content: [{ type: "text", text: "ok" }] }));
  }
}
function makeResponse(id, result) {
  return { jsonrpc: "2.0", id, result };
}
main().catch((e) => { console.error("[tsk-mcp] fatal:", e); process.exit(1); });