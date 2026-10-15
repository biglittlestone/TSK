#!/usr/bin/env node
// 收益收集器：逐轮归因 + 汇总报告。
//
//   collect.mjs --session-of <iter.json>            打印该轮会话 id（供 run-persistent.sh 导出）
//   collect.mjs --iter N --journal J --lines-before L --run R --export E --verify-rc C --title T --seconds S
//   collect.mjs --report --runs <dir> --journal <path>
import fs from "node:fs"
import path from "node:path"

const RECALL_TOK = 75 // 单次回读（tsk search / KB cat）实测中值，沿用 eval/deveco-e2e-collect.mjs

const argv = process.argv.slice(2)
function flag(name, def = undefined) {
  const i = argv.indexOf(name)
  return i >= 0 ? argv[i + 1] : def
}
const tok = (s) => Math.ceil(String(s).length / 4)

function readLines(p) {
  try { return fs.readFileSync(p, "utf8").split("\n").filter(Boolean) } catch { return [] }
}

// ---- 从事件流里找会话 id ----
function sessionIdOf(runFile) {
  for (const line of readLines(runFile)) {
    let o; try { o = JSON.parse(line) } catch { continue }
    const cand = o.sessionID ?? o.session_id ?? o.session?.id ?? o.info?.id ?? o.properties?.sessionID
    if (cand) return String(cand)
  }
  return ""
}

// ---- 该轮 journal 增量（saved_tokens）----
function journalDelta(journalPath, fromLine) {
  const lines = readLines(journalPath)
  let saved = 0, n = 0, kb = 0
  for (let i = fromLine; i < lines.length; i++) {
    let o; try { o = JSON.parse(lines[i]) } catch { continue }
    if (o.kind === "sandbox-output") { saved += o.saved_tokens || 0; n++ }
    if (o.kind === "kb-save") kb++
  }
  return { saved, n, kb }
}

// ---- 回读次数：事件流里 bash 命令匹配 tsk search / 读 KB ----
function recallCount(runFile) {
  let c = 0
  for (const line of readLines(runFile)) {
    let o; try { o = JSON.parse(line) } catch { continue }
    const s = JSON.stringify(o)
    if (/tsk search/.test(s) || /\.tsk[\\/]kb[\\/]/.test(s)) {
      if (/"bash"/.test(s) || /"command"/.test(s)) c++
    }
  }
  return c
}

// ---- 会话 input tokens：GLM 把输入记在 cache.read，input 常为 0 → 三者相加（= effective input）----
function tokensInput(exportFile) {
  let o = null
  try { o = JSON.parse(fs.readFileSync(exportFile, "utf8")) } catch { /* maybe jsonl */ }
  if (!o) {
    for (const line of readLines(exportFile)) {
      try { const j = JSON.parse(line); if (j?.info?.tokens) { o = j; break } } catch { /* skip */ }
    }
  }
  const t = o?.info?.tokens
  if (t) return (t.input || 0) + (t.cache?.read || 0) + (t.cache?.write || 0)
  return 0
}

// ================= 子命令 =================
if (argv.includes("--session-of")) {
  process.stdout.write(sessionIdOf(flag("--session-of")))
  process.exit(0)
}

if (argv.includes("--iter")) {
  const i = Number(flag("--iter"))
  const journal = flag("--journal")
  const from = Number(flag("--lines-before", "0"))
  const run = flag("--run")
  const exp = flag("--export")
  const verifyRc = Number(flag("--verify-rc", "0"))
  const title = flag("--title", "")
  const seconds = Number(flag("--seconds", "0"))

  const { saved, n, kb } = journalDelta(journal, from)
  const recalls = recallCount(run)
  const recallTok = recalls * RECALL_TOK
  const net = saved - recallTok
  const input = tokensInput(exp)
  const e2e = input > 0 ? (saved / input) * 100 : 0

  const rec = {
    iter: i, title, seconds, saved_tokens: saved, compressed: n, kb_saves: kb,
    recalls, recall_tokens: recallTok, net_tokens: net,
    session_input_tokens: input, e2e_pct: Number(e2e.toFixed(4)),
    verify_pass: verifyRc === 0,
  }
  const outDir = path.dirname(run)
  fs.writeFileSync(path.join(outDir, `iter-${i}.metrics.json`), JSON.stringify(rec, null, 2))
  console.log(`  saved ${saved} tok / 压 ${n} 次 / KB ${kb} / 回读 ${recalls}≈${recallTok} / 净 ${net} / 会话input ${input} / e2e ${e2e.toFixed(3)}% / 完成度 ${verifyRc === 0 ? "✅" : "❌"}`)
  process.exit(0)
}

if (argv.includes("--report")) {
  const runs = flag("--runs")
  const journal = flag("--journal")
  const recs = (fs.existsSync(runs) ? fs.readdirSync(runs) : [])
    .filter((f) => /^iter-\d+\.metrics\.json$/.test(f))
    .map((f) => JSON.parse(fs.readFileSync(path.join(runs, f), "utf8")))
    .sort((a, b) => a.iter - b.iter)

  const sum = (k) => recs.reduce((a, r) => a + (r[k] || 0), 0)
  const totalSaved = sum("saved_tokens"), totalRecall = sum("recall_tokens")
  const totalNet = totalSaved - totalRecall
  const totalInput = sum("session_input_tokens")
  const done = recs.filter((r) => r.verify_pass).length

  let md = "# TSK 真实案例：图书管理系统常驻优化（deveco）\n\n"
  md += `口径：saved = Σ journal \`sandbox-output.saved_tokens\`（⌈orig/4⌉−⌈kept/4⌉）· 回读 = 会话内 \`tsk search\`/读 KB 次数 × ${RECALL_TOK} tok · 净 = saved − 回读 · e2e% = saved / 该会话 input tokens · 完成度 = verify.sh（后端测试过 / 前端产 hap）。\n\n`
  md += "## 逐轮\n\n"
  md += "| 轮 | 任务 | 用时s | 压缩次数 | saved tok | 回读 | 回读tok | **净 tok** | 会话input | e2e% | 完成度 |\n"
  md += "|---|---|---|---|---|---|---|---|---|---|---|\n"
  for (const r of recs) {
    md += `| ${r.iter} | ${r.title} | ${r.seconds} | ${r.compressed} | ${r.saved_tokens} | ${r.recalls} | ${r.recall_tokens} | **${r.net_tokens}** | ${r.session_input_tokens} | ${r.e2e_pct}% | ${r.verify_pass ? "✅" : "❌"} |\n`
  }
  md += "\n## 累计\n\n"
  md += `- 轮数：${recs.length}（完成度 ${done}/${recs.length}）\n`
  md += `- TSK saved：**${totalSaved}** tok（压缩 ${sum("compressed")} 次，KB 落盘 ${sum("kb_saves")} 次）\n`
  md += `- 回读成本：${totalRecall} tok（${sum("recalls")} 次）\n`
  md += `- **净收益：${totalNet} tok**\n`
  md += `- 会话总 input：${totalInput} tok\n`
  md += `- **端到端占比：${totalInput > 0 ? ((totalSaved / totalInput) * 100).toFixed(4) : "0"}%**\n`

  // 默认写到本工作目录（<WORK>/report.md），避免 A/B 各臂互相覆盖；
  // 需要汇总到 docs/ 时用 TSK_CASE_REPORT_DIR 显式指定。
  const outDir = process.env.TSK_CASE_REPORT_DIR || path.resolve(runs, "..")
  const outPath = path.join(outDir, "report.md")
  fs.mkdirSync(path.dirname(outPath), { recursive: true })
  fs.writeFileSync(outPath, md)
  console.log(md)
  console.log(`report -> ${outPath}`)
  process.exit(0)
}

console.error("usage: collect.mjs --session-of|--iter|--report ...")
process.exit(2)
