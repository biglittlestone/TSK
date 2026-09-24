// TSK × DevEco Code 沙盒冒烟测试。
// 用 node 驱动真实插件（integration/deveco/tsk.plugin.ts）+ 真实 tsk 二进制，
// 模拟 DevEco 事件流（session.created / tool.execute.before / system.transform），
// 全部数据落在隔离的临时 TSK_HOME 与临时项目里，不碰真实 ~/.tsk。
//
// 运行：node integration/deveco/smoke.test.mjs   （需 tsk 在 PATH，或设 TSK_BIN=路径）
import { spawnSync } from "node:child_process"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"

let passed = 0
let failed = 0
const ok = (name, cond, extra = "") => {
  if (cond) {
    passed++
    console.log(`  ✓ ${name}`)
  } else {
    failed++
    console.log(`  ✗ ${name}  ${extra}`)
  }
}

// 临时沙盒：隔离 TSK_HOME + 项目 + 大文件
const base = fs.mkdtempSync(path.join(os.tmpdir(), "tsk-deveco-"))
const home = path.join(base, "home")
const proj = path.join(base, "proj")
fs.mkdirSync(path.join(home), { recursive: true })
fs.mkdirSync(path.join(proj, ".tsk", "skeletons"), { recursive: true })
const big = path.join(proj, "big.txt")
fs.writeFileSync(big, Array.from({ length: 100 }, (_, i) => `line ${i + 1}\n`).join(""))
const tskBin = process.env.TSK_BIN || "tsk"

const runTsk = (cmd, input) => {
  const parts = cmd.trim().split(/\s+/)
  const r = spawnSync(parts[0], parts.slice(1), {
    input: input ?? "",
    encoding: "utf8",
    env: { ...process.env, TSK_HOME: home },
  })
  return JSON.parse(r.stdout || "{}")
}

// mock DevEco 的 BunShell `$`（Bun.$\`tsk hook X\`.input().quiet().nothrow().json()）
const shell = (strings, ...exprs) => {
  const cmd = strings.map((s, i) => s + (exprs[i] ?? "")).join("").trim()
  const self = {
    _input: "",
    input(v) {
      self._input = v
      return self
    },
    quiet() {
      return self
    },
    nothrow() {
      return self
    },
    json() {
      return runTsk(cmd, self._input)
    },
  }
  return self
}

const loadPlugin = async () => {
  const src = fs.readFileSync(path.join(import.meta.dirname, "tsk.plugin.ts"), "utf8")
  const mod = await import("data:text/javascript;base64," + Buffer.from(src).toString("base64"))
  return mod.default
}
const writeCfg = (level) =>
  fs.writeFileSync(path.join(home, "config.yaml"), `enabled: true\noutput_compression: ${level}\n`)

// ===================== 输入压缩（骨架 / 逃逸口） =====================
console.log("== 输入压缩 ==")
writeCfg("full")
const hooks = await (await loadPlugin())({ $: shell, directory: proj })

// read 大文件 → filePath 改为骨架路径，骨架存在且内容正确
const out1 = { args: { filePath: big } }
await hooks["tool.execute.before"]({ tool: "read", sessionID: "s1", callID: "c1" }, out1)
ok("read 大文件 → filePath 被改写为骨架", out1.args.filePath !== big)
ok("骨架文件存在", fs.existsSync(out1.args.filePath))
const sk = fs.readFileSync(out1.args.filePath, "utf8")
ok("骨架头正确", sk.startsWith("[TSK SKELETON of big.txt | 100 lines total"))

// read 带 offset/limit → 逃逸口，恒放行（filePath 不变）
const out2 = { args: { filePath: big, offset: 50, limit: 10 } }
await hooks["tool.execute.before"]({ tool: "read", sessionID: "s1", callID: "c2" }, out2)
ok("read offset/limit → passthrough（filePath 不变）", out2.args.filePath === big)

// read 非 read 工具 → 不干预
const out3 = { args: { command: "ls" } }
await hooks["tool.execute.before"]({ tool: "bash", sessionID: "s1", callID: "c3" }, out3)
ok("非 read 工具不改写", out3.args.command === "ls")

// 每个用户消息轮调用 UserPromptSubmit；返回的 additionalContext 逐个（drain）冲入 system
const turnThenTransform = async (hooks, sid) => {
  await hooks.event({ event: { type: "turn", properties: { sessionID: sid } } })
  const sys = ["<base system>"]
  await hooks["experimental.chat.system.transform"]({}, { system: sys })
  return sys.join("\n")
}

// ===================== 输出压缩（system prompt 注入 ruleset） =====================
console.log("== 输出压缩 ==")
writeCfg("full")
const hooks2 = await (await loadPlugin())({ $: shell, directory: proj })
await hooks2.event({ event: { type: "session.created", properties: { sessionID: "s1" } } })
const sys = ["<original system>"]
await hooks2["experimental.chat.system.transform"]({}, { system: sys })
const joined = sys.join("\n")
ok("full → ruleset 注入 system prompt", joined.includes("TSK OUTPUT MODE ACTIVE") && joined.includes("HARD RULES"))
// 幂等：再次 transform 不重复
const count = joined.split("TSK OUTPUT MODE").length - 1
await hooks2["experimental.chat.system.transform"]({}, { system: sys })
ok("system.transform 幂等（ruleset 只注入一次）", sys.join("\n").split("TSK OUTPUT MODE").length - 1 === count)

// off → 不注入
writeCfg("off")
const hooks3 = await (await loadPlugin())({ $: shell, directory: proj })
await hooks3.event({ event: { type: "session.created", properties: { sessionID: "s1" } } })
const sysOff = ["<base>"]
await hooks3["experimental.chat.system.transform"]({}, { system: sysOff })
ok("off → 不注入 ruleset", !sysOff.join("\n").includes("TSK OUTPUT MODE"))

// lite → 注入 lite 档
writeCfg("lite")
const hooks4 = await (await loadPlugin())({ $: shell, directory: proj })
await hooks4.event({ event: { type: "session.created", properties: { sessionID: "s1" } } })
const sysLite = ["<base>"]
await hooks4["experimental.chat.system.transform"]({}, { system: sysLite })
ok("lite → 注入 lite ruleset", sysLite.join("\n").includes("(level: lite)"))

// ===================== auto 档轮数化（委托 Rust 裁决） =====================
console.log("== auto 档（第 3 轮起注入） ==")
writeCfg("auto")
const prevTurnEvents = process.env.TSK_DEVECO_TURN_EVENTS
process.env.TSK_DEVECO_TURN_EVENTS = "turn"
const hooksAuto = await (await loadPlugin())({ $: shell, directory: proj })
let autoHasRuleset = [false, false, false, false]
const rulesetHits = [] // 出现 HARD RULES 的轮次（ruleset 应恰在第 3 轮一次）
for (let i = 1; i <= 5; i++) {
  const joined = await turnThenTransform(hooksAuto, "auto-s1")
  if (i <= 4) autoHasRuleset[i - 1] = joined.includes("TSK OUTPUT MODE ACTIVE")
  if (joined.includes("HARD RULES")) rulesetHits.push(i)
}
ok("auto 轮1 → 不注入", !autoHasRuleset[0])
ok("auto 轮2 → 不注入", !autoHasRuleset[1])
ok("auto 轮3 → 注入 ruleset", autoHasRuleset[2])
ok("auto 轮4 → 仍注入（reinforce）", autoHasRuleset[3])
ok("auto ruleset 恰在第 3 轮注入一次（不重复）", rulesetHits.length === 1 && rulesetHits[0] === 3)
// 空转：session 只有 1-2 轮时 short-session，events.log 仍正确记账（无注入但无残留）
if (prevTurnEvents === undefined) delete process.env.TSK_DEVECO_TURN_EVENTS
else process.env.TSK_DEVECO_TURN_EVENTS = prevTurnEvents

// ===================== compact → resume（历史上线） =====================
console.log("== compact → resume ==")
writeCfg("full")
// 构造一个可被 snapshot::build 解析的 transcript（JSONL）
const trPath = path.join(base, "transcript.jsonl")
fs.writeFileSync(
  trPath,
  [
    JSON.stringify({ type: "user", message: { content: "remember the db schema" } }),
    JSON.stringify({ type: "assistant", message: { content: "ok, dbschema: users(id, name)" } }),
  ].join("\n") + "\n",
)
process.env.TSK_DEVECO_TRANSCRIPT = trPath
const hooksC = await (await loadPlugin())({ $: shell, directory: proj })
await hooksC.event({ event: { type: "session.compacted", properties: { sessionID: "comp-s1" } } })
const sysCompact = ["<base>"]
await hooksC["experimental.chat.system.transform"]({}, { system: sysCompact })
const compacted = sysCompact.join("\n")
ok("compact → 注入 PreCompact 锚点保护标记", compacted.includes("context window is being compacted"))
ok("compact → 注入 RESUME（决策/技能摘要）", compacted.includes("RESUME:"))
ok("compact → resume-snapshot.json 已落盘", fs.existsSync(path.join(home, "comp-s1", "resume-snapshot.json")))

fs.rmSync(base, { recursive: true, force: true })
console.log(`\n结果: ${passed} 通过 / ${failed} 失败`)
process.exit(failed > 0 ? 1 : 0)
