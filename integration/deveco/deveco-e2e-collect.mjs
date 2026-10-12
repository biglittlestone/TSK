// deveco 端到端收集：从一次跑完的 deveco 会话目录算实际净收益（journal 记账 + 日志回读 + 完成度）。
// 用法：bun integration/deveco/deveco-e2e-collect.mjs <run 目录>
// 输出：压缩次数 / saved tok / 回读次数 / 净收益 / 完成度（hap 或 BUILD SUCCESSFUL）
import { readFileSync, existsSync } from "node:fs"
import { spawnSync } from "node:child_process"

const dir = process.argv[2] || "."
const RECALL_TOK = 75 // 单次回读（tsk search 片段）实测中值

// 1. TSK journal → 实际压缩记账
const journalPath = `${dir}/.tsk/injections.jsonl`.replace(/\\/g, "/")
let saved = 0, nComp = 0, kbSaves = 0
if (existsSync(journalPath)) {
  for (const line of readFileSync(journalPath, "utf8").split("\n").filter(Boolean)) {
    try {
      const x = JSON.parse(line)
      if (x.kind === "sandbox-output") { saved += x.saved_tokens || 0; nComp++ }
      if (x.kind === "kb-save") kbSaves++
    } catch {}
  }
}

// 2. 回读：模型实际发起的 tsk search / cat|read .tsk/kb（日志可感知）
const runLog = `${dir}/run.log`.replace(/\\/g, "/")
const log = existsSync(runLog) ? readFileSync(runLog, "utf8") : ""
const recalls = (log.match(/tsk search|\bcat\b[^\n]*\.tsk[\\/]kb|\bread\b[^\n]*\.tsk[\\/]kb/gi) || []).length

// 3. 完成度：hap 产出 / BUILD SUCCESSFUL / 工程目录
const haps = spawnSync("find", [dir, "-name", "*.hap"], { encoding: "utf8", maxBuffer: 8 * 1024 * 1024 }).stdout
  .trim().split("\n").filter(Boolean).filter(p => !p.includes("node_modules"))
const built = /BUILD SUCCESSFUL/.test(log)
const done = haps.length > 0

const recallTok = recalls * RECALL_TOK
const net = saved - recallTok
const status = done ? `✅ 完成（${haps[0].split("/").pop()}）`
  : built ? "⚠️ 构建成功但无 hap 产物"
  : `❌ 未完成（无 hap${built ? "" : " / 无 BUILD SUCCESSFUL"}）`

console.log(`== deveco 端到端净收益（实际记账）==`)
console.log(`目录   : ${dir}`)
console.log(`压缩   : ${nComp} 次 | saved ${saved} tok`)
console.log(`回读   : ${recalls} 次 ≈ ${recallTok} tok（单次 ${RECALL_TOK}）`)
console.log(`净收益 : ${net} tok`)
console.log(`完成度 : ${status}`)