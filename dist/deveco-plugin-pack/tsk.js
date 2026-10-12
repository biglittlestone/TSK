// /tsk 命令脚本：TSK 收益统计（opencode/deveco 脚本命令，Bun 执行，不经模型）。
// 用法：/tsk          → 本会话（最近会话）
//       /tsk all      → 全部会话
//       /tsk clean    → 重置记账（清空统计，重新累计）
// 由 install.bat 复制到 DevEco 的命令目录。也可手动复制到 .opencode/command/tsk.js。
const fs = require("fs")
const { execSync } = require("child_process")
function run(cmd) { try { return execSync(cmd, { encoding: "utf8", shell: true }) } catch (e) { return null } }
const arg = String(process.argv[2] || "").trim().toLowerCase()
let flags = []
if (arg === "all") flags = ["--all"]             // 全部会话
else if (arg === "clean") flags = ["--clean"]    // 重置记账
// 默认（含 this/session）：本会话（最近会话）
const args = ["report", ...flags]
const home = process.env.USERPROFILE || process.env.HOME || ""
const fixed = home + "\\.tsk\\bin\\tsk.exe"
let out = null
if (fs.existsSync(fixed)) out = run('"' + fixed + '" ' + args.join(" ")) // 固定安装路径（install.bat 装的）
if (out === null) out = run("tsk " + args.join(" ")) // PATH fallback
if (out) { process.stdout.write(out); process.exit(0) }
process.stderr.write("tsk report failed: tsk not found. Run install.bat or add tsk to PATH.\n")
process.exit(1)
