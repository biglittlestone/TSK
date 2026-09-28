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
const ctxDir = path.join(proj, ".tsk", "ctx")

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
  ok("超大输出落盘 KB + 取回指针", out.output.includes(".tsk/ctx/k") && fs.readdirSync(ctxDir).some((f) => f.endsWith(".txt")))
  ok("journal 记 sandbox-output（含原始/蒸馏规模）", fs.existsSync(journ) && fs.readFileSync(journ, "utf8").includes("sandbox-output"))
}

// 小输出（≤1600 字符）放行不动
{
  const out = { output: "ok\n" }
  await hooks["tool.execute.after"]({ tool: "bash", sessionID: "s1", callID: "c5", args: { command: "pwd" } }, out)
  ok("小输出放行不动", out.output === "ok\n")
}

fs.rmSync(base, { recursive: true, force: true })
console.log(`\n结果: ${passed} 通过 / ${failed} 失败`)
process.exit(failed > 0 ? 1 : 0)