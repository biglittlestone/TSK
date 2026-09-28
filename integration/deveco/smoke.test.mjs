// TSK × DevEco Code 沙盒冒烟测试（对齐 V7 插件设计）。
// 目标：验证 opencode 平台上 TSK 的「真实价值机制」—— tool.execute.after 输出压缩
// （大 Bash/工具输出 → 蒸馏只留 关键错误/状态行+结尾 + 超大落盘 KB + 取回指针），
// 这正是 Claude Code 原生做不到的那条路。并覆盖：read 无损直通、净不模拟器拦截、小输出放行。
//
// 运行：bun integration/deveco/smoke.test.mjs   （插件是 TS + Bun.spawn，须用 bun；tsk 在 PATH）
// 隔离：process.env.TSK_HOME = 临时家目录，插件 spawn 的 tsk 继承它，不碰真实 ~/.tsk。
//
// 注：tool.execute.after 的输出压缩是纯 JS（不 spawn tsk），故本节全确定性；其余钩子按需测。
import { spawnSync } from "node:child_process"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"

let passed = 0
let failed = 0
const ok = (name, cond, extra = "") => {
  if (cond) passed++
  else {
    failed++
    console.log(`  ✗ ${name}  ${extra}`)
    return
  }
  console.log(`  ✓ ${name}`)
}

const base = fs.mkdtempSync(path.join(os.tmpdir(), "tsk-deveco-"))
const home = path.join(base, "home")
const proj = path.join(base, "proj")
fs.mkdirSync(path.join(home), { recursive: true })
fs.mkdirSync(path.join(proj), { recursive: true })
process.env.TSK_HOME = home // 让插件 Bun.spawn(["tsk",…]) 继承隔离
process.env.TSK_JOURNAL_DIR = path.join(proj, ".tsk")
const big = path.join(proj, "big.txt")
fs.writeFileSync(big, Array.from({ length: 100 }, (_, i) => `line ${i + 1}\n`).join(""))

const loadPlugin = async () => {
  const src = fs.readFileSync(path.join(import.meta.dirname, "tsk.plugin.ts"), "utf8")
  const mod = await import("data:text/javascript;base64," + Buffer.from(src).toString("base64"))
  return mod.default // {id, async server({directory})}
}

const hooks = await (await loadPlugin()).server({ directory: proj })
const journ = path.join(proj, ".tsk", "injections.jsonl")
const ctxDir = path.join(proj, ".tsk", "kb")

// ===================== read 无损直通 =====================
console.log("== read 无损（V7：不再骨架改写） ==")
{
  const out = { args: { filePath: big, offset: 0, limit: 100 } }
  await hooks["tool.execute.before"]({ tool: "read", sessionID: "s1", callID: "c1" }, out)
  ok("read → filePath 直通不变（无损）", out.args.filePath === big)
}
{
  const out = { args: { filePath: big } }
  await hooks["tool.execute.before"]({ tool: "read", sessionID: "s1", callID: "c1b" }, out)
  ok("read 无 offset → 也直通（无损）", out.args.filePath === big)
}

// ===================== 永不启动模拟器/设备 =====================
console.log("== 净不模拟器 / 网络拦截 ==")
{
  const out = { args: { command: "hdc start emulator && deveco run" } }
  await hooks["tool.execute.before"]({ tool: "bash", sessionID: "s1", callID: "c2" }, out)
  // 拦截改写为拦截告知（消息内会回显原命令，故断言前缀而非“不含原串”）
  ok("模拟器/设备命令被拦截改写", out.args.command.startsWith('echo "[TSK 拦截'))
}
{
  const out = { args: { command: "curl https://x" } }
  await hooks["tool.execute.before"]({ tool: "bash", sessionID: "s1", callID: "c2b" }, out)
  ok("curl/wget 网络被拦截", out.args.command.includes("[TSK 路由"))
}
{
  const out = { args: { command: "ls -la" } }
  await hooks["tool.execute.before"]({ tool: "bash", sessionID: "s1", callID: "c2c" }, out)
  ok("普通命令放行", out.args.command === "ls -la")
}

// ===================== tool.execute.after 输出压缩（真实价值机制） =====================
console.log("== tool.execute.after 输出压缩（opencode 独有、Claude 原生做不到）==")
// 构造 ~6KB 含大量错误行的构建输出：>800*2 触发；含错误行 → verbose；>5KB 且命令命中 intent → 落盘 KB
{
  const err = Array.from({ length: 40 }, (_, i) => `ERROR build-${i}: cannot resolve module 'x${i}'`).join("\n")
  const noise = Array.from({ length: 2000 }, (_, i) => `step-${String(i).padStart(4, "0")} init worker done queue ${i % 7} take ${i % 3}`).join("\n")
  const raw = `npm run build\n${noise}\n${err}\n----------\nBUILD FAILED with 40 errors` // ~85KB
  const out = { output: raw }
  await hooks["tool.execute.after"](
    { tool: "bash", sessionID: "s1", callID: "c4", args: { command: "analyze build log for errors" } },
    out,
  )
  ok("大输出被改写（含 TSK 沙箱标记）", out.output.includes("[TSK 沙箱") && out.output.includes("原文 ") && out.output.includes("tok"))
  ok("蒸馏保留关键错误行（错误感知）", out.output.includes("── 关键错误/状态行 ──") && out.output.includes("cannot resolve module"))
  ok("蒸馏保留结尾（BUILD 结果）", out.output.includes("BUILD FAILED") || out.output.includes("── 结尾 ──"))
  ok("超大输出落盘 KB + 取回指针", out.output.includes(".tsk/kb/k") && fs.readdirSync(ctxDir).some((f) => f.endsWith(".txt")))
  ok("journal 记 sandbox-output（含原始/蒸馏规模）", fs.existsSync(journ) && fs.readFileSync(journ, "utf8").includes("sandbox-output"))
}

// 小输出（≤1600 字符）放行不动
{
  const out = { output: "ok\n" }
  await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c5", args: { command: "pwd" } }, out)
  ok("小输出放行不动", out.output === "ok\n")
}

// ===================== rtk 式：逐命令语义压缩（符合工具短结果格式 + 无损落盘） =====================
console.log("== rtk 式逐命令压缩（ls / git status / test）==")
{
  // ls：很多文件 → 计数 + 前 N 项 + more，仍是目录列表形态
  const raw = Array.from({ length: 60 }, (_, i) => `${i < 20 ? "drwxr-xr-x" : "-rw-r--r--"} user grp ${1000 + i} Sep 27 10:00 ${"mod" + String(i).padStart(2, "0")}`).join("\n")
  const out = { output: raw }
  await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c6", args: { command: "ls -la" } }, out)
  ok("ls → 计数+前N项（目录形态）", /dirs?, \d+ files?:/.test(out.output) && /mod00/.test(out.output))
  ok("ls → 超 CAP 有 +more", out.output.includes("more"))
  ok("ls → 无损落盘 KB 可回取（含 60 项原文）", out.output.includes(".tsk/kb/k") && fs.readFileSync(fs.readdirSync(ctxDir).filter((f) => f.endsWith(".txt")).map((f) => path.join(ctxDir, f)).find((p) => (fs.readFileSync(p, "utf8").split("\n").length >= 60)), "utf8").split("\n").length >= 60)
}
{
  // git status --short → 按状态分组 + 数量（像 git 短格式）；raw >1600 过 size gate
  const raw = Array.from({ length: 140 }, (_, i) => i % 3 === 0 ? `M  mod${i}.ts` : i % 3 === 1 ? `?? untracked${i}.js` : `A  added${i}.ts`).join("\n")
  const out = { output: raw }
  await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c7", args: { command: "git status --short" } }, out)
  ok("git status → 按状态分组+数量", /M \(\d+\)/.test(out.output) && /A \(\d+\)/.test(out.output) && /.. \(\d+\)/.test(out.output))
  ok("git status → 无损落盘", out.output.includes(".tsk/kb/k"))
}
{
  // git log → 保留每 commit hash+subject（oneline 语义）
  const raw = Array.from({ length: 40 }, (_, i) => `commit ${"deadbeef" + String(i).padStart(8, "0")}\nAuthor: u\nDate:  2026-09-2${i % 9}\n\n    fix: patch ${i} thing\n\n    some long body line that should be dropped ${"x".repeat(30)}`).join("\n\n")
  const out = { output: raw }
  await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c9", args: { command: "git log --stat" } }, out)
  ok("git log → 每commit一行 hash+subject（oneline 语义）", /40 commits:/.test(out.output) && /deadbeef0000.*fix: patch 0/.test(out.output))
  ok("git log → 丢正文 long body", !out.output.includes("long body line that should be dropped"))
  ok("git log → 无损落盘", out.output.includes(".tsk/kb/k"))
}
{
  // git diff → 保留 per-file + hunk 头 + 变更统计，丢 +/-正文
  const body = Array.from({ length: 300 }, (_, i) => ` ${"a".repeat(40)}`).join("\n")
  const raw = `diff --git a/src/a.ts b/src/a.ts\nnew file mode 100644\n@@ -0,0 +1,200 @@\n${body}\ndiff --git a/src/b.ts b/src/b.ts\n@@ -10,5 +10,4 @@\n 1 file changed, 205 insertions(+), 5 deletions(-)`
  const out = { output: raw }
  await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c10", args: { command: "git diff HEAD" } }, out)
  ok("git diff → 保留 per-file 名", /2 files:/.test(out.output) && /src\/a.ts/.test(out.output))
  ok("git diff → 保留 hunk 头", /@@ -0,0 \+1,200 @@/.test(out.output))
  ok("git diff → 丢 +/- 正文", !out.output.includes("a".repeat(40)))
  ok("git diff → 无损落盘", out.output.includes(".tsk/kb/k"))
}
{
  // vitest → 报告状：Tests 汇总 + 前 N 条失败
  const fails = Array.from({ length: 40 }, (_, i) => `✗ tests/foo${i}.test.ts > case ${i}\n  AssertionError: expected ${i} to equal ${9 - i}`).join("\n")
  const raw = ` RUN  v2\n ${JSON.stringify({ numTotalTests: 40, numPassedTests: 0, numFailedTests: 40 })}\n\n${fails}\n Test Files  1 failed (1)\n Tests  0 passed | 40 failed (40)`
  const out = { output: raw }
  await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c8", args: { command: "npx vitest run" } }, out)
  ok("test → 报告状（保留 Tests 行 + failures 上限）", /Tests\s+0 passed|10 created|failures \(\d+ shown\)/.test(out.output) || /failed/.test(out.output))
  ok("test → failures 不超过 CAP_ERR+收尾", out.output.length < (raw.length - 20))
  ok("test → 无损落盘", out.output.includes(".tsk/kb/k"))
}
// —— 新增：grep / wc / find / du / df / ps（rtk 式，格式保持 + 无损） ——
{
  const raw = Array.from({ length: 70 }, (_, i) => `src/f${i}.ts:${12 + i}:export const v${i} = ${i};`).join("\n")
  const out = { output: raw }
  await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c11", args: { command: "rg -n 'export const' src" } }, out)
  ok("grep → 按文件分组+行号", /grep \(\d+ files, \d+ matches\):/.test(out.output) && /src\/f0\.ts \(1\):/.test(out.output) && /12: export const v0/.test(out.output))
  ok("grep → 无损落盘", out.output.includes(".tsk/kb/k"))
}
{
  const raw = Array.from({ length: 90 }, (_, i) => `123 456 7890 common/dir/${String.fromCharCode(97 + (i % 10))}.ts`).join("\n")
  const out = { output: raw }
  await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c12", args: { command: "wc -l a/b/c/*.ts" } }, out)
  ok("wc → 剥公共前缀", out.output.includes("90 项") && /公共前缀"common\/dir\/"已剥/.test(out.output) && !out.output.includes("common/dir/a.ts") && out.output.includes("a.ts"))
  ok("wc → 无损落盘", out.output.includes(".tsk/kb/k"))
}
{
  const raw = Array.from({ length: 140 }, (_, i) => `/repo/src/d${i % 4}/file${String(i).padStart(3, "0")}.ts`).join("\n")
  const out = { output: raw }
  await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c13", args: { command: "find . -name '*.ts'" } }, out)
  ok("find → 计数+N项", /files:\n/.test(out.output) && out.output.includes("file000.ts") && out.output.includes("more paths"))
  ok("find → 无损落盘", out.output.includes(".tsk/kb/k"))
}
{
  const raw = `12K\t./node_modules\n${Array.from({ length: 80 }, (_, i) => `${i + 1}M\t./src/file${i}.ts`).join("\n")}\n500M\ttotal`
  const out = { output: raw }
  await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c14", args: { command: "du -sh *" } }, out)
  ok("du → 保留项+total", /du \(\d+ 项\):/.test(out.output) && /total: 500M/.test(out.output))
}
{
  const raw = `Filesystem     1K-blocks    Used Available Use% Mounted on\n${Array.from({ length: 45 }, (_, i) => `/dev/sda${i}  1048576 ${i * 10} ${999999 - i * 10} ${i}% /mnt/d${i}`).join("\n")}`
  const out = { output: raw }
  await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c15", args: { command: "df -h" } }, out)
  ok("df → 表头+封顶", out.output.startsWith("[TSK") && out.output.includes("Filesystem") && out.output.includes("more"))
  ok("ps → 表头+封顶", await (async () => { const o = { output: `PID USER COMMAND\n${Array.from({ length: 120 }, (_, i) => `${1000 + i} acetone cmd${i} --flag=${"z".repeat(40)} ${i}`).join("\n")}` }; await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c16", args: { command: "ps aux" } }, o); return o.output.includes("PID") && o.output.includes("more"); })())
}

fs.rmSync(base, { recursive: true, force: true })
console.log(`\n结果: ${passed} 通过 / ${failed} 失败`)
process.exit(failed > 0 ? 1 : 0)