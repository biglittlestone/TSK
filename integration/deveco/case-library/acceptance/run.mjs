// 独立验收套件（agent 无关）：按 SPEC.md 的 REST 契约，直接对运行中的服务做 HTTP 断言。
// 不依赖 agent 自写的测试（那可能被削弱/删改），是「任务完成度」的独立信号。
//
// 用法：node acceptance/run.mjs <seedDir>
// 退出码 0 = 全部通过。
import { spawn } from "node:child_process"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"

const seed = path.resolve(process.argv[2] || ".work/seed")
const serverDir = path.join(seed, "server")
if (!fs.existsSync(path.join(serverDir, "server.js"))) {
  console.log("acceptance: server/server.js 不存在 → 未实现")
  process.exit(1)
}

const PORT = 3400 + Math.floor(Math.random() * 500)
const DB = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "libacc-")), "acc.db")
const proc = spawn("bun", ["server.js"], { cwd: serverDir, env: { ...process.env, PORT: String(PORT), DB_PATH: DB }, stdio: "ignore", shell: true })

// 端口自适应：实现可能不认 PORT（SPEC 早期未强制），回退探测 3000
let base = `http://127.0.0.1:${PORT}`
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
async function reachable(b) { try { const r = await fetch(b + "/books"); return r.ok || r.status === 404 } catch { return false } }
async function up() {
  for (let i = 0; i < 60; i++) {
    if (await reachable(base)) return true
    const alt = "http://127.0.0.1:3000"
    if (base !== alt && await reachable(alt)) { base = alt; return true }
    await sleep(250)
  }
  return false
}
async function api(method, p, body) {
  const init = { method }
  if (body !== undefined) { init.body = JSON.stringify(body); init.headers = { "content-type": "application/json" } }
  const r = await fetch(base + p, init)
  let j = null; try { j = await r.json() } catch { /* non-json */ }
  return { status: r.status, json: j }
}

const results = []
const ok = (name, cond, extra = "") => results.push({ name, cond: !!cond, extra })

try {
  if (!await up()) { console.log("acceptance: 服务未起来"); proc.kill(); process.exit(1) }

  // ---- 书目：增 / 查 / 分页信封 ----
  const created = await api("POST", "/books", { title: "验收书A", author: "作者A", isbn: "978-ACC-1", category: "科幻" })
  ok("POST /books 建书 201", created.status === 201, `got ${created.status}`)
  const bid = created.json?.id
  ok("POST /books 缺字段 400", (await api("POST", "/books", { title: "x" })).status === 400)

  const list = await api("GET", "/books")
  ok("GET /books 分页信封 {total,page,size,items}", list.json && "total" in list.json && "items" in list.json && "page" in list.json && "size" in list.json, JSON.stringify(list.json)?.slice(0, 80))
  const search = await api("GET", "/books?q=" + encodeURIComponent("验收书A"))
  ok("GET /books?q= 能搜到", (search.json?.items || []).some((b) => b.id === bid), JSON.stringify(search.json)?.slice(0, 80))
  ok("GET /books/:id 未知 404", (await api("GET", "/books/999999")).status === 404)

  // ---- 借出 / 归还 ----
  const loan = await api("POST", "/loans", { bookId: bid, member: "读者X" })
  ok("POST /loans 借出 201", loan.status === 201, `got ${loan.status}`)
  const lid = loan.json?.id

  const due = loan.json?.dueDate
  const today = new Date().toISOString().slice(0, 10)
  const diffDays = due ? Math.round((new Date(due + "T00:00:00Z") - new Date(today + "T00:00:00Z")) / 86400000) : null
  ok("借期 = 今天 + 14 天（opt-4）", diffDays === 14, `due=${due} today=${today} diff=${diffDays}`)

  const book = await api("GET", `/books/${bid}`)
  // SPEC 只要求书置为 borrowed（borrower 字段可选）→ 容忍借书人记在书或借阅记录上
  const borrowerOnBook = book.json?.borrower ?? book.json?.borrowedBy ?? book.json?.borrowerName
  ok("借出后书状态 borrowed（opt-3 前置）", book.json?.status === "borrowed" || book.json?.state === "borrowed", JSON.stringify(book.json)?.slice(0, 90))
  ok("借书人已登记（书或借阅记录）", borrowerOnBook === "读者X" || loan.json?.member === "读者X" || loan.json?.borrower === "读者X", JSON.stringify(book.json)?.slice(0, 90))

  const dup = await api("POST", "/loans", { bookId: bid, member: "读者Y" })
  ok("同一本再借 → 409（opt-3）", dup.status === 409, `got ${dup.status}`)

  const loans = await api("GET", "/loans")
  const loanItems = Array.isArray(loans.json) ? loans.json : (loans.json?.items || [])
  const mine = loanItems.find((l) => l.id === lid || l.bookId === bid || l.book_id === bid)
  ok("GET /loans 带 active/overdue 标记", mine && mine.active === true && mine.overdue === false, JSON.stringify(mine)?.slice(0, 90))

  ok("归还 200", (await api("POST", `/loans/${lid}/return`)).status === 200)
  ok("重复归还 409", (await api("POST", `/loans/${lid}/return`)).status === 409)
  const after = await api("GET", `/books/${bid}`)
  ok("归还后书回到 in", after.json?.status === "in", JSON.stringify(after.json)?.slice(0, 80))

  ok("DELETE /books/:id", (await api("DELETE", `/books/${bid}`)).status === 200)
} catch (e) {
  ok("套件执行无异常", false, String(e))
} finally {
  proc.kill()
}

const failed = results.filter((r) => !r.cond)
for (const r of results) console.log(`  ${r.cond ? "✓" : "✗"} ${r.name}${r.cond ? "" : "  → " + r.extra}`)
console.log(`acceptance: ${results.length - failed.length}/${results.length} 通过`)
process.exit(failed.length ? 1 : 0)
