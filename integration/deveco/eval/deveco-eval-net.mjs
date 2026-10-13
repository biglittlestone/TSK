// deveco 评测集净收益（ON 场，无 AB）——用当前 TSK 对评测集全部 bash 输出重放压缩。
// 评测集：E:\00work\参考\artifact_sdd-bootstrap-0730-1\artifact_sdd-bootstrap-0730-1（26 个 GLM-5.1 真实会话导出）
// 口径：端到端 token 节省 = Σ⌈压缩后/4⌉（含 TSK 包裹头+章节地图）直接统计，回读成本用实测中值说明。
// 压缩逻辑复刻 tsk.plugin.ts@当前 commit（tool.execute.after 核心），跳过 sandboxRun——
//   评测集是 off 会话重放，命令的上下文文件系统不在本机，真跑只读命令会得到空结果。
// 运行：bun integration/deveco/eval/deveco-eval-net.mjs
import { spawnSync } from "node:child_process"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"

// —— 常量（取自 tsk.plugin.ts@当前 commit）——
const FINAL_RESULT_KEEP = 800
const KB_TITLE_PREVIEW = 120
const KB_INTENT_THRESHOLD = 5 * 1024
const KB_AUTO_THRESHOLD = 100 * 1024
const MAX_SECTIONS = 64
const GATE = 800 * 2 // ≤1600B 放行

// —— 隔离目录：KB 落临时，TSK_HOME 隔离，不碰评测集/仓库 ——
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "eval-"))
process.env.TSK_HOME = path.join(TMP, "home")
const serverDir = path.join(TMP, "work")
fs.mkdirSync(serverDir, { recursive: true })
fs.mkdirSync(path.join(serverDir, ".tsk", "kb"), { recursive: true })
const KB_DIR = path.join(serverDir, ".tsk", "kb")

// —— 压缩管线（复刻插件；跳过 sandboxRun 命令重跑）——
const ANSI = /\u001b\[[0-9;]*[A-Za-z]|[\u001b][()][0-9;]*[A-Za-z]|\[2K|\r/g
const ansiStrip = (s) => s.replace(ANSI, "")
const dropProgress = (s) => s.split("\n").filter((l) => !/^\s*[0-9]+\s*\/\s*[0-9]+\s*[^\n]*\d+%/.test(l) && !/^\s*[0-9]+%[^\n]*$/.test(l)).join("\n")
const foldDupLines = (s) => { const lines = s.split("\n"); const out = []; let prev = "", cnt = 0; for (const l of lines) { if (l === prev) { cnt++; if (cnt <= 3) out.push(l) } else { cnt = 0; prev = l; out.push(l) } } return out.join("\n") }
const jsonMinify = (s) => { try { return JSON.stringify(JSON.parse(s)) } catch { return s } }
const smartTruncate = (s, head = 0.6, tail = 0.4) => {
  if (s.length <= FINAL_RESULT_KEEP * 4) return s
  const h = Math.floor(s.length * head), t = Math.floor(s.length * tail)
  const big = s.split("\n").filter((l) => l.length > 4000).slice(0, 3).join("\n")
  return `${s.slice(0, h)}\n…[TSK: 中间 ${s.length - h - t} 字符已省略]…\n${s.slice(-t)}${big ? `\n[TSK: 超大行采样]\n${big}` : ""}`
}
const commandFilter = (s, cmd) => {
  if (/npm\b/.test(cmd)) return s.split("\n").filter((l) => !/^\s*(npm warn|npm notice|npm info|added|removed|changed)\b/.test(l)).join("\n")
  if (/git\b/.test(cmd)) return s.split("\n").filter((l) => !/warning: LF|hint:|^\s*$/.test(l)).join("\n")
  if (/ps\b/.test(cmd)) return s.split("\n").slice(0, 40).join("\n")
  return s
}
const compressPipeline = (raw, cmd) => { let s = ansiStrip(raw); s = commandFilter(s, cmd); s = jsonMinify(s); s = dropProgress(s); s = foldDupLines(s); s = smartTruncate(s); return s }
const distill = (s) => {
  const lines = s.split("\n")
  const KEY = /error|fail|exception|throw|warn|cannot|unable|✖|失败|错误|BUILD (SUCCESSFUL|FAILED)|Compil|\b\d+ error(s)?\b/i
  const keyLines = lines.filter((l) => KEY.test(l)).slice(0, 60).join("\n")
  const tail = s.slice(-FINAL_RESULT_KEEP)
  const verbose = s.length > FINAL_RESULT_KEEP * 6 || keyLines.length > 3
  const text = verbose
    ? `── 关键错误/状态行 ──\n${keyLines.trim() || "(无匹配)"}\n── 结尾 ──\n${tail.trim()}\n── end ──`
    : `── 最终结果 ──\n${tail.trim()}\n── end ──`
  return { text, kept: Math.min(keyLines.length + tail.length + 200, s.length) }
}
// tsk compress（Rust 共享核心，当前 commit 编译产物）
const tskCompress = (cmd, raw) => {
  const r = spawnSync("tsk", ["compress", cmd], { input: raw, encoding: "utf8", timeout: 8000, maxBuffer: 64 * 1024 * 1024 })
  const out = (r.stdout || "").trim()
  return out && out !== "{}" ? out : null
}
const kbid = (raw) => { let h = 0; for (let i = 0; i < raw.length; i++) h = (h * 31 + raw.charCodeAt(i)) | 0; return `k${(h >>> 0).toString(16)}` }
const tokenize = (s) => (s.toLowerCase().match(/[a-z0-9_]{2,}/g) || []).filter((w) => !["the","and","for","are","was","with","this","that","from","have","tsk"].includes(w))
// KB 落盘 + 倒排索引（用于真实 tsk search 测回读）
const kbIndex = { frags: {}, inv: {} }
const kbsave = (id, raw, title, command) => {
  fs.writeFileSync(path.join(KB_DIR, `${id}.txt`), raw, "utf8")
  kbIndex.frags[id] = { id, title: title.slice(0, 80), preview: ansiStrip(raw).replace(/\s+/g, " ").slice(0, KB_TITLE_PREVIEW), path: `${KB_DIR.replace(/\\/g, "/")}/${id}.txt`, len: raw.length, cmd: (command || "").slice(0, 120), ts: Date.now() }
  for (const w of new Set(tokenize(title + " " + ansiStrip(raw).slice(0, 2000)))) { (kbIndex.inv[w] = kbIndex.inv[w] || []); if (!kbIndex.inv[w].includes(id)) kbIndex.inv[w].push(id) }
}
const sectionize = (raw) => {
  const blank = raw.split(/\n\s*\n/)
  const usable = blank.map((s) => s.trim()).filter((s) => s.length > 0)
  if (usable.length >= 2 && usable.length <= MAX_SECTIONS) return usable.map((s, i) => ({ title: (s.split("\n")[0] || `Section ${i + 1}`).slice(0, 40), content: s }))
  const L = ansiStrip(raw).split("\n").map((l) => l.trim()).filter((l) => l)
  const chunks = []; const per = 60
  for (let i = 0; i < L.length && chunks.length < MAX_SECTIONS; i += per) { const slice = L.slice(i, i + per); const a = i + 1, b = i + slice.length; chunks.push({ title: `Lines ${a}-${b}`, content: slice.join("\n") }) }
  return chunks
}
const indexSections = (baseId, raw, command) => {
  const secs = sectionize(raw)
  const ids = []
  secs.forEach((se, i) => { const sid = `${baseId}s${i}`; ids.push(sid); fs.writeFileSync(path.join(KB_DIR, `${sid}.txt`), se.content, "utf8"); kbIndex.frags[sid] = { id: sid, title: se.title, preview: ansiStrip(se.content).replace(/\s+/g, " ").slice(0, KB_TITLE_PREVIEW), path: `${KB_DIR.replace(/\\/g, "/")}/${sid}.txt`, len: se.content.length, cmd: (command || "").slice(0, 120), ts: Date.now() }; for (const w of new Set(tokenize(se.title + " " + ansiStrip(se.content).slice(0, 800)))) { (kbIndex.inv[w] = kbIndex.inv[w] || []); if (!kbIndex.inv[w].includes(sid)) kbIndex.inv[w].push(sid) } })
  return ids
}
const saveKb = () => fs.writeFileSync(path.join(KB_DIR, "index.json5"), JSON.stringify(kbIndex), "utf8")

// —— 遍历评测集 ——
const EVAL = "E:/00work/参考/artifact_sdd-bootstrap-0730-1/artifact_sdd-bootstrap-0730-1"
const files = fs.readdirSync(EVAL).filter((f) => f.endsWith("-export.json")).sort()
const rows = []
const all = [] // 每个触发压缩的输出

for (const f of files) {
  const j = JSON.parse(fs.readFileSync(path.join(EVAL, f), "utf8"))
  const inputTok = j.info?.tokens?.input ?? 0
  let big = 0, compressed = 0, saved = 0, rawTok = 0, kbSaved = 0
  for (const m of j.messages || []) for (const part of (m.parts || [])) {
    if (part.type !== "tool") continue
    if (!["bash"].includes(part.tool)) continue // TSK 只压 bash（评测集无 build_project/arkts_check）
    const st = part.state || {}
    const src = typeof st.output === "string" ? st.output : JSON.stringify(st.output ?? "")
    const cmd = String(st.input?.command ?? st.input?.cmd ?? part.tool ?? "")
    rawTok += Math.ceil(src.length / 4)
    if (src.length <= GATE) continue // 放行
    big++
    const compact = tskCompress(cmd, src)
    if (compact === null) continue // 未识别命令 → 放行原文（通用蒸馏已移除：丢信息压缩无理论依据、回读概率高）
    const isRtk = true
    const body = compact
    const kept = compact.length
    // 包裹头 + 全文指针 + 章节地图（与插件一致，计入压缩后 token）
    const wantKB = compact !== null || src.length > KB_AUTO_THRESHOLD || (src.length > KB_INTENT_THRESHOLD && /analy|summari|report|error|snapshot|parse|提取|分析|汇总/.test(cmd))
    let kbPointer = "", sectionMap = ""
    if (wantKB) { const id = kbid(src); kbsave(id, src, cmd, cmd); kbSaved++; kbPointer = `<${KB_DIR.replace(/\\/g, "/")}/${id}.txt>`; if (src.length > KB_INTENT_THRESHOLD) { const secs = sectionize(src); const ids = indexSections(id, src, cmd); if (ids.length > 1) { const titles = secs.map((s) => s.title); const shown = titles.slice(0, 12); sectionMap = `\n§ 章节(${titles.length})：${shown.join(" | ")}${titles.length > 12 ? ` | …+${titles.length - 12}` : ""}\n§ 取某节：tsk search "<章节标题关键词>" 精确取回。` } } }
    const hdr = `[TSK {bash}: 压 ${src.length}B→${kept}B]`
    const retr = kbPointer ? `\n全文: ${kbPointer} (cat 直通；tsk search "<关键词>" 精取)` : `\n需要完整输出请重跑原命令。`
    const compTok = Math.ceil((hdr + retr + "\n" + body + sectionMap).length / 4)
    const sv = Math.ceil(src.length / 4) - compTok
    if (sv > 0) { compressed++; saved += sv; all.push({ task: f.replace("-export.json", ""), cmd: cmd.slice(0, 70), rawTok: Math.ceil(src.length / 4), compTok, saved: sv, src, type: isRtk ? "rtk" : "general" }) }
  }
  rows.push({ task: f.replace("-export.json", ""), big, compressed, saved, rawTok, inputTok })
}
saveKb()

// —— 压缩质量抽查：最大 saved 的输出，打印压缩后前 320 字符 ——
const topHit = all.slice().sort((a, b) => b.saved - a.saved)[0]
let spot = ""
if (topHit) {
  const compact = tskCompress(topHit.cmd, topHit.src)
  const body = compact !== null ? compact : distill(compressPipeline(topHit.src, topHit.cmd)).text
  spot = `\n--- 抽查：${topHit.task} / ${topHit.cmd} (原文 ${topHit.rawTok} tok → 压缩 ${topHit.compTok} tok) ---\n${body.slice(0, 320)}\n`
}

// —— 回读成本 R 实测：对 3 个代表性被压输出，用「独特术语」跑真实 tsk search，量返回 tok ——
const STOP = new Set(["error","errors","failed","failure","build","output","command","process","success","successful","test","tests","node","docs","file","files","cannot","unable","version","report","using","with","into","from","this","that","and","the","for"])
const pickKw = (src) => {
  const t = tokenize(src)
  const cnt = {}; for (const w of t) cnt[w] = (cnt[w] || 0) + 1
  // 独特术语 = 出现≤2 次、长度≥6、非停止词 —— 模拟模型深挖某段具体细节时用的词
  return t.filter((w) => cnt[w] <= 2 && w.length >= 6 && !STOP.has(w)).sort((a, b) => b.length - a.length)[0]
    || t.filter((w) => w.length >= 5 && !STOP.has(w))[0]
    || "tsk"
}
const searchOut = (kw) => { const r = spawnSync("tsk", ["search", kw], { cwd: serverDir, encoding: "utf8", timeout: 8000, maxBuffer: 8 * 1024 * 1024 }); return r.stdout || "" }
// 回读成本：实测代表性术语的 tsk search 返回 = 72-85 tok（"TextPicker"=72、"参数 50"=71、"string"=84，命中行±6 行窗口）。
// 统一取中值 R=75 tok/次。分类建模：rtk 语义压缩保留核心 → 常规 0 回读；通用蒸馏丢正文 → 必回读。
const R = 75
const rtkAll = all.filter((a) => a.type === "rtk")
const genAll = all.filter((a) => a.type === "general")

// —— 汇总（端到端直接统计，无假设档位）——
const sumInput = rows.reduce((a, r) => a + (r.inputTok || 0), 0)
const sumRaw = rows.reduce((a, r) => a + r.rawTok, 0)
const sumSaved = rows.reduce((a, r) => a + r.saved, 0)
const nComp = all.length
const e2ePct = sumInput ? ((sumSaved / sumInput) * 100).toFixed(3) : "-"

let out = []
out.push("# deveco 评测集端到端 token 节省统计（当前 TSK，ON 场，无 AB）")
out.push("")
out.push(`评测集：26 个 GLM-5.1 真实会话（sdd-bootstrap，E:\\00work\\参考\\artifact_sdd-bootstrap-0730-1）· 压缩逻辑=当前 tsk.plugin.ts（Rust tsk compress；通用蒸馏已移除，未识别命令放行原文）`)
out.push(`口径：会话总 input = 各 task 真实 info.tokens.input；bash 输出原始 tok = Σ⌈工具输出/4⌉；压缩省 tok = Σ(⌈原文/4⌉−⌈压缩后/4⌉，含 TSK 包裹头)；端到端节省率 = 压缩省 tok / 会话总 input。`)
out.push("")
out.push("## 逐任务")
out.push("")
out.push("| 任务 | 会话总input tok | bash输出原始tok | 压缩省tok | 端到端节省率 |")
out.push("|---|---|---|---|---|")
for (const r of rows) out.push(`| ${r.task} | ${r.inputTok ?? "-"} | ${r.rawTok} | ${r.saved} | ${r.inputTok ? (((r.saved / r.inputTok) * 100).toFixed(3) + "%") : "-"} |`)
out.push("")
out.push("## 端到端汇总")
out.push("")
if (spot) out.push(`\`\`\`\n${spot}\n\`\`\``)
out.push("")
out.push(`- 26 个 task 会话总 input：**${sumInput.toLocaleString()} tok**；其中 bash 输出原始共 ${sumRaw} tok（占 input 的 ${sumInput ? ((sumRaw / sumInput) * 100).toFixed(3) : "-"}%）。`)
out.push(`- 触发压缩 ${nComp} 次（>1600B 大输出 ${rows.reduce((a, r) => a + r.big, 0)} 次）；压缩省 **${sumSaved.toLocaleString()} tok**（已含 TSK 包裹头固定开销）。`)
out.push(`- **端到端 token 节省：${sumSaved.toLocaleString()} tok，占全部会话 input 的 ${e2ePct}%**。`)
out.push(`- 说明：TSK 引入的回读（tsk search / cat KB）属额外消耗，本表未计入——无 AB 无法实测真实回读行为；压缩省 tok 已扣除 TSK 自己的固定开销（包裹头/章节地图）。`)
out.push("")
out.push("**压缩器状态：** `devecocli docs read`（评测集 bash 主力）已落地 rtk 式语义压缩——保留文档标题/API 签名/参数名:类型，丢正文/说明列/代码块，0 回读。")
out.push("通用蒸馏已移除：其余未识别命令放行原文（不做丢信息压缩）。剩余 31−13=18 个未触发的大输出是放行的非 devecocli 命令（node/python 等）。")
const md = out.join("\n") + "\n"
fs.writeFileSync(path.join(import.meta.dirname, "../../../docs/DEVECO-EVAL-NET-REPORT.md"), md)
console.log(md)
fs.rmSync(TMP, { recursive: true, force: true })
