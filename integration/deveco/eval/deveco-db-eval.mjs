// deveco 任务级端到端收益评测（从 deveco-local.db 提取真实会话，重放 TSK 压缩）。
// 用法：bun integration/deveco/eval/deveco-db-eval.mjs [deveco-local.db 路径]
// 默认 db：codegenie-cli-benchmark 的 deveco-feat-20260701-data。
// 输出：每任务 saved tok + 端到端百分比（saved / 会话总 input），落 docs/DEVECO-DB-EVAL-REPORT.md
import { Database } from "bun:sqlite"
import { spawnSync } from "node:child_process"

const dbPath = process.argv[2] || "E:/00work/参考/codegenie-cli-benchmark-master-copy/tmp/deveco-feat-20260701-data/deveco/deveco-local.db"
const tok = (c) => Math.ceil(c.length / 4)
const tskCompress = (cmd, raw) => { const r = spawnSync("tsk", ["compress", cmd], { input: raw, encoding: "utf8", timeout: 8000 }); const o = (r.stdout || "").trim(); return o && o !== "{}" ? o : null }

const db = new Database(dbPath, { readonly: true })
const sessions = db.query("SELECT id, title, directory, tokens_input FROM session").all()
const parts = db.query("SELECT session_id, data FROM part").all()

// 会话 id → bash 工具调用列表
const bashBy = new Map()
for (const p of parts) {
  let d
  try { d = JSON.parse(p.data) } catch { continue }
  if (d?.type !== "tool" || d?.tool !== "bash") continue
  const st = d.state || {}
  let out = st.output ?? ""
  if (typeof out !== "string") out = JSON.stringify(out)
  const cmd = String(st.input?.command ?? "")
  if (!bashBy.has(p.session_id)) bashBy.set(p.session_id, [])
  bashBy.get(p.session_id).push({ cmd, output: out })
}

function caseOf(s) {
  const m = (s.directory || "").match(/task-([\w-]+)/)
  if (m) return m[1]
  return (s.title || "").split(" ").slice(0, 4).join(" ") + "…"
}

const byCase = new Map()
let totInput = 0, totSaved = 0, totTrig = 0, totSess = 0
for (const s of sessions) {
  const key = caseOf(s)
  if (!byCase.has(key)) byCase.set(key, { n: 0, input: 0, bashTok: 0, saved: 0, trig: 0 })
  const e = byCase.get(key)
  e.n++; e.input += s.tokens_input || 0; totInput += s.tokens_input || 0; totSess++
  for (const b of bashBy.get(s.id) || []) {
    e.bashTok += tok(b.output)
    if (b.output.length <= 1600) continue
    const c = tskCompress(b.cmd, b.output)
    if (c !== null && c !== b.output) { const sv = tok(b.output) - tok(c); e.saved += sv; e.trig++; totSaved += sv; totTrig++ }
  }
}
const pct = (a, b) => (b ? ((a / b) * 100).toFixed(4) + "%" : "-")
const rows = [...byCase.entries()].sort((a, b) => b[1].saved - a[1].saved)

let out = []
out.push("# deveco 任务级端到端收益（db 真实会话重放）")
out.push("")
out.push(`数据源：${dbPath.split("/").pop()} · ${totSess} 会话 / ${byCase.size} 任务 · 总 input ${(totInput / 1e6).toFixed(1)}M tok`)
out.push(`口径：saved = 重放当前 TSK 对 >1600B bash 输出压缩省下的 tok；端到端% = saved / 该任务会话总 input（tokens_input 实测）。`)
out.push("")
out.push(`**总体：触发 ${totTrig} 次，总 saved ${totSaved.toLocaleString()} tok = ${pct(totSaved, totInput)} 端到端**`)
out.push("")
out.push("## 任务级（按 saved 降序）")
out.push("")
out.push("| 任务 | 会话 | 总input tok | bash输出tok | 触发 | saved tok | 端到端% |")
out.push("|---|---|---|---|---|---|---|")
for (const [k, e] of rows) out.push(`| ${k} | ${e.n} | ${e.input.toLocaleString()} | ${e.bashTok} | ${e.trig} | ${e.saved.toLocaleString()} | ${pct(e.saved, e.input)} |`)
out.push("")
out.push("**观察：** 收益集中在 bash 输出占比高的任务（grep/find/ls/cat 命令级压缩）。")
out.push("0/负收益任务 = bash 输出 <1600B 散碎（放行）或压缩文本短于包裹头的边界。")
const md = out.join("\n") + "\n"
Bun.write("E:/00work/TSK/docs/DEVECO-DB-EVAL-REPORT.md", md)
console.log(md)
