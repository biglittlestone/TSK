// read-CCR 行为测试：直接驱动 tsk.plugin.ts 的 tool.execute.after，验证开关与类型门控。
// 插件顶层常量在模块加载时求值，故本脚本按当前 TSK_READ_CCR 值跑一遍；由外部跑两次（0/1）。
// 运行：TSK_READ_CCR=0 bun integration/deveco/eval/read-ccr-test.mjs && TSK_READ_CCR=1 bun integration/deveco/eval/read-ccr-test.mjs
import fs from "node:fs"
import os from "node:os"
import path from "node:path"

const FLAG = process.env.TSK_READ_CCR === "1"
const pluginSrc = fs.readFileSync(path.join(import.meta.dirname, "..", "tsk.plugin.ts"), "utf8")
const base = fs.mkdtempSync(path.join(os.tmpdir(), "readccr-"))
process.env.TSK_HOME = path.join(base, "home")

let pass = 0, fail = 0
const ok = (name, cond) => { if (cond) { pass++; console.log("  ✓ " + name) } else { fail++; console.log("  ✗ " + name) } }

const mod = await import("data:text/javascript;base64," + Buffer.from(pluginSrc).toString("base64"))
const hooks = await mod.default.server({ directory: fs.mkdtempSync(path.join(base, "proj-")) })

const bigJson = JSON.stringify({ total: 1100, entries: Array.from({ length: 300 }, (_, i) => ({ id: i, title: "书" + i, author: "作者" + i, isbn: "978-" + i })) })
const bigTs = "export function f() {\n" + "  const x = 1;\n".repeat(900) + "}\n"
const fire = async (filePath, content, cid) => {
  const out = { output: content }
  await hooks["tool.execute.after"]({ tool: "read", sessionID: "s1", callID: cid, args: { filePath } }, out)
  return out.output
}

console.log(`== TSK_READ_CCR=${FLAG ? "1" : "0"} ==`)
const jsonOut = await fire("/p/reference/books.json", bigJson, "c1")
const tsOut = await fire("/p/app/entry/src/main/ets/pages/Index.ets", bigTs, "c2")
const jsOut = await fire("/p/server/server.js", bigTs, "c3")
const smallOut = await fire("/p/small.json", '{"a":1}', "c4")

if (!FLAG) {
  ok("开关关：read 一律直通", jsonOut === bigJson && tsOut === bigTs && jsOut === bigTs && smallOut === '{"a":1}')
} else {
  ok("数据文件(.json) 被压缩", jsonOut !== bigJson && jsonOut.includes("[TSK {read}"))
  ok("压缩后更短", jsonOut.length < bigJson.length)
  ok("带 KB 指针（可无损取回）", jsonOut.includes(".tsk/kb/"))
  ok("源码(.ets) 直通不改写", tsOut === bigTs)
  ok("源码(.js) 直通不改写", jsOut === bigTs)
  ok("小文件(≤1600B) 直通", smallOut === '{"a":1}')
}
console.log(`结果: ${pass} 通过 / ${fail} 失败`)
process.exit(fail ? 1 : 0)
