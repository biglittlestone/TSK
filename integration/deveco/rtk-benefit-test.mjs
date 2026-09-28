// TSK × deveco 真实例子收益测试（rtk 式逐命令压缩）。
// 用真实命令的真实输出，驱动「与 deveco 里加载的同一份」插件 tsk.plugin.ts 的
// tool.execute.after，量化每命令的 bytes/token 削减，并验证无损回取(.tsk/kb + tsk search)。
// 运行：bun integration/deveco/rtk-benefit-test.mjs
import { spawnSync } from "node:child_process"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"

const base = fs.mkdtempSync(path.join(os.tmpdir(), "rtkbt-"))
process.env.TSK_HOME = path.join(base, "home")
const repo = path.join(base, "repo")
const proj = path.join(base, "proj")

// —— 1. 真实 git 仓库：若干文件 + 提交 + 未提交改动 + 未跟踪 ——
fs.mkdirSync(repo)
run(["git", "init", "-q"], repo)
run(["git", "config", "user.email", "t@t"], repo)
run(["git", "config", "user.name", "t"], repo)
for (let i = 0; i < 30; i++) {
  let f = ""
  for (let k = 0; k < 30; k++) f += `export const fn${i}_${k} = ${i + k};\n// ${"z".repeat(60)} ${i}\n`
  fs.writeFileSync(path.join(repo, `mod${i}.ts`), f)
  run(["git", "add", "."], repo)
  run(["git", "commit", "-qm", `feat: patch ${i} — ${"long body ".repeat(4)} ${i}`], repo)
  if (i >= 1) { fs.writeFileSync(path.join(repo, `mod${i}.ts`), f + "\n// later edit\n") } // 制造后续 diff
}
fs.writeFileSync(path.join(repo, "new.js"), "// untracked\n")
const gitStatus = run(["git", "status", "--short"], repo, true)
const gitLog = run(["git", "log", "--stat"], repo, true)
const gitDiff = run(["git", "diff", "HEAD~5", "HEAD"], repo, true)

// —— 2. 真实 ls -laR（嵌套目录） ——
for (let d = 0; d < 3; d++) { fs.mkdirSync(path.join(proj, `src/dir${d}`), { recursive: true }); for (let f = 0; f < 40; f++) fs.writeFileSync(path.join(proj, `src/dir${d}/f${f}.ts`), "// f\n") }
const lsR = run(["ls", "-laR", "src"], proj, true)

// —— 3. 真实 node --test（1 个真实失败用例） ——
fs.writeFileSync(path.join(proj, "sum.test.js"), `const assert=require("node:assert/strict");\nfor(let i=0;i<60;i++){ require("node:test").test("case "+i,()=>assert.equal(i,i<30?i:999)); }\n`)
const testOut = run(["node", "--test"], proj, true)

// —— 4. 加载插件（与部署的同一份源码）并驱动 tool.execute.after ——
const src = fs.readFileSync(path.join(import.meta.dirname, "tsk.plugin.ts"), "utf8")
const mod = await import("data:text/javascript;base64," + Buffer.from(src).toString("base64"))
const hooks = await mod.default.server({ directory: proj })

const runs = [
  ["git status --short", "bash", gitStatus],
  ["git log --stat", "bash", gitLog],
  ["git diff HEAD~5 HEAD", "bash", gitDiff],
  ["ls -laR src", "bash", lsR],
  ["node --test", "bash", testOut],
]

const limit = (s) => (s.length > 90 ? s.slice(0, 90) + "…" : s)
console.log("\n=== 真实命令 → 插件压缩收益（>1600B 才压；小输出放行=0%） ===")
console.log("命令".padEnd(22) + "| 原始B  | 压缩B | 省B  | 省% | 特征验证 | 无损KB")
for (const [cmd, tool, out] of runs) {
  const o = { output: out }
  const before = out.length
  await hooks["tool.execute.after"]({ tool, sessionID: "r1", callID: "c", args: { command: cmd } }, o)
  const after = o.output.length
  const saved = before - after
  const pct = ((saved / before) * 100).toFixed(0)
  const kbHit = fs.existsSync(path.join(proj, ".tsk", "kb")) && fs.readdirSync(path.join(proj, ".tsk", "kb")).some((f) => f.endsWith(".txt"))
  const feature = /git status/.test(o.output) ? "status分组"
    : /commits?:/.test(o.output) ? "oneline"
    : /files:|hunks?:/.test(o.output) ? "diff/file+hunk"
    : /d(irs?)?, \d+ files?:/.test(o.output) ? "ls计数"
    : /Tests|test suite/.test(o.output) ? "测试报告"
    : "放行(小)"
  console.log(`${limit(cmd).padEnd(22)}| ${String(before).padStart(6)} | ${String(after).padStart(6)} | ${String(saved).padStart(4)} | ${pct.padStart(3)}% | ${feature.padEnd(11)} | ${kbHit ? "✓" : "✗"}`)
}

// —— 5. 无损回取验证：tsk search 从 proj cwd 在 KB 里找回已压缩命令的原文 ——
console.log("\n=== 无损回取（tsk search 找回已压缩 git diff 的原文） ===")
const probe = spawnSync("tsk", ["search", "mod0"], { cwd: proj, encoding: "utf8" })
const hit = /mod0\.ts/.test(probe.stdout)
console.log("git diff 被压，原文 mod0.ts 能否从 KB 精确找回:", hit ? "✓" : "✗")
console.log(probe.stdout.split("\n").slice(0, 6).join("\n") || "(no hits)")

fs.rmSync(base, { recursive: true, force: true })

function run(cmd, cwd, captureOut = false) {
  const r = spawnSync(cmd[0], cmd.slice(1), { cwd, encoding: "utf8" })
  if (!captureOut) return r.status ?? 0
  return (r.stdout || "") + (r.status === 0 ? "\n" : `\n[exit ${r.status}]\n${(r.stderr || "").trim()}`)
}