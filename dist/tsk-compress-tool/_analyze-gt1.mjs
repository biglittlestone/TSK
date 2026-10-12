// 分析：为什么没有 >1% 端到端？每 task bash占比×压缩率，低压缩率 task 的 bash 构成
import { readFileSync } from "node:fs"
import { spawnSync } from "node:child_process"
const tok = (c) => Math.ceil(c.length / 4)
const tskCompress = (cmd, raw) => { const r = spawnSync("tsk", ["compress", cmd], { input: raw, encoding: "utf8", timeout: 8000 }); const o = (r.stdout || "").trim(); return o && o !== "{}" ? o : null }
const sessions = JSON.parse(readFileSync("C:/Users/Administrator/AppData/Local/Temp/db-bash.json", "utf8"))
function caseOf(s) { const m = (s.directory || "").match(/task-([\w-]+)/); if (m) return m[1]; return (s.title || "").split(" ").slice(0, 4).join(" ") + "…" }

const byCase = new Map()
for (const s of sessions) {
  const key = caseOf(s)
  if (!byCase.has(key)) byCase.set(key, { n: 0, input: 0, bashTok: 0, saved: 0, big: 0, unregBig: [], cmdDist: {} })
  const e = byCase.get(key); e.n++; e.input += s.tokens_input || 0
  for (const b of s.bash) {
    const t = tok(b.output); e.bashTok += t
    const first = (b.cmd.split(/\s/)[0] || "?").slice(0, 14)
    e.cmdDist[first] = (e.cmdDist[first] || 0) + 1
    if (b.output.length <= 1600) continue
    e.big++
    const c = tskCompress(b.cmd, b.output)
    if (c !== null && c !== b.output) e.saved += tok(b.output) - tok(c)
    else e.unregBig.push({ cmd: first, len: b.output.length })
  }
}
console.log("任务".padEnd(26) + "bash/input | 压缩率 | 端到端% | 大输出 | 未触发大输出(前3)")
for (const [k, e] of [...byCase.entries()].sort((a, b) => (b[1].bashTok / b[1].input) - (a[1].bashTok / a[1].input)).slice(0, 12)) {
  const ratio = e.input ? ((e.bashTok / e.input) * 100).toFixed(2) + "%" : "-"
  const rate = e.bashTok ? ((e.saved / e.bashTok) * 100).toFixed(1) + "%" : "-"
  const pct = e.input ? ((e.saved / e.input) * 100).toFixed(3) + "%" : "-"
  const unreg = [...e.unregBig].sort((a, b) => b.len - a.len).slice(0, 3).map(u => `${u.cmd}(${u.len}B)`).join(" ") || "—"
  console.log(`${k.padEnd(26)} ${ratio.padStart(8)} | ${rate.padStart(6)} | ${pct.padStart(8)} | ${String(e.big).padStart(4)} | ${unreg}`)
}
