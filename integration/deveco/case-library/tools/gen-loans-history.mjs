#!/usr/bin/env node
// 生成 reference/loans_history.json（借阅历史）——配方 A 的第二个大数据源。
// 用法：node tools/gen-loans-history.mjs <books.json> <输出路径>
// 确定性（固定 seed），可复现。字段与语义与 books.json 关联：15 位读者池、1100 本书池。
import fs from "node:fs"

const [, , booksPath, outPath] = process.argv
if (!booksPath || !outPath) { console.error("用法: node gen-loans-history.mjs <books.json> <out.json>"); process.exit(1) }

// 确定性随机
let _s = 42
const rnd = () => (_s = (_s * 1664525 + 1013904223) >>> 0) / 0xffffffff
const randInt = (a, b) => a + Math.floor(rnd() * (b - a + 1))

const data = JSON.parse(fs.readFileSync(booksPath, "utf8"))
const entries = data.entries || data
const readerPool = [...new Set(entries.map((b) => b.borrower).filter(Boolean))]
const bookIds = entries.map((b) => b.id)

// 日期工具
const DAY = 86400000
const d0 = new Date("2023-01-01T00:00:00+08:00").getTime()
const dEnd = new Date("2026-10-15T00:00:00+08:00").getTime()
const fmt = (t) => {
  const d = new Date(t)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`
}
const today = new Date("2026-10-15T00:00:00+08:00").getTime()

const N = 9000
const loans = []
for (let i = 0; i < N; i++) {
  let borrowedAt = d0 + Math.floor(rnd() * (dEnd - d0))
  let dueDate = borrowedAt + 14 * DAY
  const roll = rnd()
  let status, returnedAt = null, dueDateStr = fmt(dueDate)
  if (roll < 0.60) {
    status = "returned"
    returnedAt = borrowedAt + randInt(1, 30) * DAY
  } else if (roll < 0.85) {
    status = "active"
  } else {
    status = "overdue"
  }
  // 埋数据问题（供审计轮挖）：
  // - ~2% dueDate 三位年畸形
  // - ~1% bookId 越界（0 或 9999）
  // - ~1% borrower 为空
  // - ~1% returnedAt 早于 borrowedAt（时间倒置）
  // - ~0.5% borrowedAt 在未来
  let bookId = bookIds[randInt(0, bookIds.length - 1)]
  let borrower = readerPool[randInt(0, readerPool.length - 1)]
  const q = rnd()
  if (q < 0.02) dueDateStr = `20${randInt(2, 3)}-01-01`.replace(/^(20\d)-(0\d)-\d/, "$1-01-01") // 变三位年
  if (q >= 0.02 && q < 0.03) bookId = rnd() < 0.5 ? 0 : 9999
  if (q >= 0.03 && q < 0.04) borrower = ""
  if (q >= 0.04 && q < 0.05 && status === "returned") returnedAt = borrowedAt - randInt(1, 10) * DAY
  if (q >= 0.05 && q < 0.055) borrowedAt = dEnd + randInt(1, 60) * DAY // 未来借出

  loans.push({
    id: i + 1,
    bookId,
    borrower,
    borrowedAt: fmt(borrowedAt),
    dueDate: dueDateStr,
    ...(returnedAt ? { returnedAt: fmt(returnedAt) } : {}),
    status,
  })
}

const body = JSON.stringify(loans, null, 1)
const wrapped = `{\n  "generated": "loans history for TSK case-library",\n  "count": ${N},\n  "readerPoolSize": ${readerPool.length},\n  "entries": ${body}\n}\n`
fs.writeFileSync(outPath, wrapped, "utf8")
console.log(`生成 ${N} 条 -> ${outPath} (${(fs.statSync(outPath).size / 1024).toFixed(0)} KB)`)
console.log(`读者池 ${readerPool.length} 人 | 书池 ${bookIds.length} 本 | 埋点: 2% 三位年/1% 越界bookId/1% 空borrower/1% 时间倒置/0.5% 未来借出`)
