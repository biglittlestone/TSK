// 提取 cat / export 大输出样例，分析内容结构（决定保留策略）
import { readFileSync } from "node:fs"
const sessions = JSON.parse(readFileSync("C:/Users/Administrator/AppData/Local/Temp/db-bash.json", "utf8"))
const catBig = [], exportBig = [], otherBig = []
for (const s of sessions) for (const b of s.bash) {
  if (b.output.length <= 1600) continue
  const first = (b.cmd.split(/\s/)[0] || "?").toLowerCase()
  const rec = { cmd: b.cmd.slice(0, 80), len: b.output.length, head: b.output.slice(0, 250), tail: b.output.slice(-200) }
  if (first === "cat" || first === "gc") catBig.push(rec)
  else if (first === "export") exportBig.push(rec)
  else otherBig.push(rec)
}
const show = (arr, n, title) => {
  console.log(`\n=== ${title}（${arr.length} 个，前 ${Math.min(n, arr.length)}）===`)
  for (const r of arr.slice(0, n)) {
    console.log(`\n[${r.len}B] ${r.cmd}`)
    console.log(`  头: ${r.head.replace(/\n/g, "⏎")}`)
    console.log(`  尾: ${r.tail.replace(/\n/g, "⏎")}`)
  }
}
show(catBig, 4, "cat 大输出")
show(exportBig, 4, "export 大输出")
show(otherBig.filter(o => /cd|rm|head|tail|sed/.test(o.cmd)), 2, "cd/rm/head/tail/sed 大输出")
