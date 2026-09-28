// TSK × DevEco Code 适配插件 —— ctx_execute 式沙箱 + 完整输出压缩管线 + 文件化知识库。
//
// 目标（Think in Code）：让 LLM 写命令/代码去分析数据，原始数据（日志/快照/API 响应）
// 留在沙箱并落盘到文件知识库，上下文只得到「蒸馏后的最终结果」。read 输入无损。
//
// 四层：
//  1. 沙箱执行（核心）—— 拦 bash/build 工具，把命令在「清理环境 + 硬上限」的子进程里跑；
//     原始 stdout 落盘，只把 console.log/蒸馏后的最终结果 喂给模型。清洗 ~70 个危险环境变量，
//     16MB 流式硬上限，超限杀进程树。deveco 的 tool.execute.before 无法阻断 agent 原始执行，
//     故以「沙箱重跑结果替换上下文里的输出」实现（read 类命令幂等；build 用管线的真实输出）。
//  2. 文件化知识库 —— 大输出（>5KB 带 intent / >100KB 自动）整篇落文件，JSON 倒排索引化；
//     检索按权重(BM25 简化)融合 RRF 取 top，只回章节「标题 + 120 字符预览」。不用 SQLite。
//  3. Hook 路由强制 —— PreToolUse 拦 curl/wget/内联 HTTP(改写为提示)、WebFetch(deny 处理端)。
//  4. Session Continuity —— 事件写 <cwd>/.tsk/events.jsonl；compact 前建快照；恢复时注入。
//
// 约束：单文件（deveco 加载），`void` 引用全清，惰性取 node:fs，服用 Bun.spawn，全程 fail-open。
// 记账：.<cwd>/.tsk/injections.jsonl（报告用）+ .<cwd>/.tsk/ctx/（原始落盘 + index.json5）。
const FINAL_RESULT_KEEP = 800         // 最终只保留尾部多少字符（构建结论/末尾错误）
const HARD_CAP_BYTES = 16 * 1024 * 1024 // 沙箱 stdout 硬上限（超限杀进程树）
const KB_INTENT_THRESHOLD = 5 * 1024  // 大输出且带 intent → 入知识库
const KB_AUTO_THRESHOLD = 100 * 1024  // 超过此自动入知识库
const CAP_LIST = 20                    // rtk 式：列表类命令保留前 N 项
const CAP_ERR = 20                     // 测试/构建：前 N 条失败
const CAP_WARN = 10
const KB_TITLE_PREVIEW = 120          // 检索只回标题 + 120 字符预览
const SANDBOX_TOOLS = new Set(["bash", "build_project", "arkts_check"])
const READONLY_CMD = /^(cat|ls|find|grep|head|tail|sed|awk|wc|env|printenv|echo|pwd|which|type|file|stat|du|df|tree)\b/

// —— 环境清洗：约 70 个危险/敏感环境变量，从子进程.env 中剔除 ——
const ENV_BLACKLIST = [
  "TSK_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY", "AZURE_OPENAI_API_KEY", "GOOGLE_API_KEY",
  "GEMINI_API_KEY", "DEEPSEEK_API_KEY", "MISTRAL_API_KEY", "OPENROUTER_API_KEY", "COHERE_API_KEY",
  "AI21_API_KEY", "VOYAGE_API_KEY", "REPLICATE_API_TOKEN", "HUGGINGFACE_TOKEN", "HF_TOKEN",
  "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN", "GOOGLE_APPLICATION_CREDENTIALS",
  "GOOGLE_SERVICE_ACCOUNT", "AZURE_CLIENT_ID", "AZURE_CLIENT_SECRET", "AZURE_TENANT_ID",
  "GCP_SA_KEY", "GCLOUD_PROJECT", "GCP_PROJECT",
  "GITHUB_TOKEN", "GITLAB_TOKEN", "BITBUCKET_TOKEN", "NPM_TOKEN", "NODE_AUTH_TOKEN", "GITHUB_PAT",
  "DOCKER_AUTH_CONFIG", "KUBE_CONFIG", "KUBECONFIG", "AWS_PROFILE",
  "DATABASE_URL", "DB_URL", "MONGO_URI", "MONGODB_URI", "REDIS_URL", "PGHOST", "PGDATABASE", "PGUSER", "PGPASSWORD",
  "SENDGRID_API_KEY", "TWILIO_ACCOUNT_SID", "TWILIO_AUTH_TOKEN", "STRIPE_API_KEY", "PAYMENT_API_KEY",
  "SUPABASE_KEY", "FIREBASE_KEY", "JSON_KEY", "SERVICE_ACCOUNT_KEY", "PRIVATE_KEY", "SECRET_KEY", "SECRET",
  "PASSWORD", "PASS", "TOKEN", "ACCESS_TOKEN", "AUTH_TOKEN", "API_TOKEN", "SSH_PRIVATE_KEY", "PASSPHRASE",
  "JITSI_APP_ID", "AUTHORIZATION", "X_API_KEY", "HTTP_AUTHORIZATION",
  "DEVECO_TOKEN", "DEVECO_API_KEY", "ARK_API_KEY", "VOLC_API_KEY",
].map((v) => v.toLowerCase())
function cleanEnv() {
  const out: Record<string, string> = {}
  for (const [k, v] of Object.entries(process.env)) {
    if (v == null) continue
    const kl = k.toLowerCase()
    if (ENV_BLACKLIST.includes(kl)) continue
    if (kl.includes("key") || kl.includes("token") || kl.includes("secret") || kl.includes("password")) continue
    out[k] = v
  }
  return out
}

// 惰性 node:fs
const fs = async () => { try { return await import("node:fs") } catch { return undefined } }
const kbd = (cwd: string) => `${cwd}/.tsk/kb`
const kbfs = async (cwd: string) => {
  const f = await fs(); if (!f) return undefined
  const d = kbd(cwd); f.mkdirSync(d, { recursive: true }); return { f, d }
}

export default {
  id: "tsk-deveco",
  async server({ directory }: any) {
    const cwd = directory ?? ""
    const journalPath = (process.env.TSK_JOURNAL_DIR || `${cwd}/.tsk`) + `/injections.jsonl`
    const eventsPath = `${cwd}/.tsk/events.jsonl`
    const goldens = new Map<string, { distilled: string; raw: string }>() // callID -> sandbox run result
    const readCount = new Map<string, number>() // file->read次数（首读压缩/回读还原全文 escape-hatch）

    const journal = async (row: any) => {
      try { const f = await fs(); if (!f) return; f.mkdirSync(`${cwd}/.tsk`, { recursive: true }); f.appendFileSync(journalPath, JSON.stringify({ ts: Date.now(), ...row }) + "\n") } catch {}
    }

    // —— 沙箱执行：清理环境 + 硬上限 + 流式捕获 ——
    const sandboxRun = async (cmd: string, runCwd: string) => {
      const env = cleanEnv()
      try {
        const proc = Bun.spawn(["bash", "-c", cmd], { cwd: runCwd, env, stdout: "pipe", stderr: "pipe" })
        const reader = proc.stdout.getReader()
        const dec = new TextDecoder()
        let out = ""
        let over = false
        for (;;) {
          const { done, value } = await reader.read()
          if (done) break
          out += dec.decode(value, { stream: true })
          if (out.length > HARD_CAP_BYTES) { over = true; try { proc.kill() } catch {}; break }
        }
        // tsk → Rust 核心桥（曾被误删，必须存在）
    const tskHook = async (hookName: string, body: any) => {
      try {
        const proc = Bun.spawn(["tsk", "hook", hookName], { stdin: "pipe", stdout: "pipe", stderr: "pipe" })
        proc.stdin.write(JSON.stringify(body)); proc.stdin.end()
        const timeout = new Promise((_, rej) => { const t = setTimeout(() => { try { proc.kill() } catch {}; rej(new Error("tskHook-timeout-4s")) }, 4000) })
        const raw: Uint8Array = await Promise.race([new Response(proc.stdout).arrayBuffer(), timeout])
        const errBuf = await new Response(proc.stderr).arrayBuffer().catch(() => new Uint8Array(0))
        if (errBuf.byteLength) await journal({ kind: "tskhook-stderr", hook: hookName, err: new TextDecoder().decode(errBuf).slice(0, 200) }).catch(() => {})
        const o = JSON.parse(new TextDecoder().decode(raw)); return (o?.hookSpecificOutput ?? {}) as any
      } catch (e: any) {
        await journal({ kind: "tskhook-error", hook: hookName, err: String((e && e.message) || e).slice(0, 200) }).catch(() => {})
        return {}
      }
    }

    return { code: over ? -9 : proc.exitCode, out, over }
      } catch {
        return { code: -1, out: "", over: false }
      }
    }

    // ———— 输出压缩管线 ————
    // tskCompress 委托 Rust 核心（共享收益引擎），替代插件内联 compactByCommand。
    const tskCompress = async (cmd: string, raw: string): Promise<string | null> => {
      try {
        const proc = Bun.spawn(["tsk", "compress", cmd], { stdin: "pipe", stdout: "pipe", stderr: "pipe" })
        proc.stdin.write(raw); proc.stdin.end()
        const timeout = new Promise((_, rej) => { const t = setTimeout(() => { try { proc.kill() } catch {}; rej(new Error("compress-timeout-8s")) }, 8000) })
        const buf: Uint8Array = await Promise.race([new Response(proc.stdout).arrayBuffer(), timeout])
        const out = new TextDecoder().decode(buf).trim()
        return out && out !== "{}" ? out : null   // CLI 无命中打印 `{}`（hook passthrough 约定）→ 视为未匹配
      } catch { return null }
    }
    const ANSI = /\u001b\[[0-9;]*[A-Za-z]|[\u001b][()][0-9;]*[A-Za-z]|\[2K|\r/g
    const ansiStrip = (s: string) => s.replace(ANSI, "")
    const dropProgress = (s: string) =>
      s.split("\n").filter((l) => !/^\s*[0-9]+\s*\/\s*[0-9]+\s*[^\n]*\d+%/.test(l) && !/^\s*[0-9]+%[^\n]*$/.test(l)).join("\n")
    const foldDupLines = (s: string) => {
      const lines = s.split("\n"); const out: string[] = []; let prev = "", cnt = 0
      for (const l of lines) { if (l === prev) { cnt++; if (cnt <= 3) out.push(l) } else { cnt = 0; prev = l; out.push(l) } }
      return out.join("\n")
    }
    const jsonMinify = (s: string) => { try { return JSON.stringify(JSON.parse(s)) } catch { return s } }
    const smartTruncate = (s: string, head = 0.6, tail = 0.4) => {
      if (s.length <= FINAL_RESULT_KEEP * 4) return s
      const h = Math.floor(s.length * head), t = Math.floor(s.length * tail)
      // 超大行采样
      const big = s.split("\n").filter((l) => l.length > 4000).slice(0, 3).join("\n")
      return `${s.slice(0, h)}\n…[TSK: 中间 ${s.length - h - t} 字符已省略]…\n${s.slice(-t)}${big ? `\n[TSK: 超大行采样]\n${big}` : ""}`
    }
    const commandFilter = (s: string, cmd: string) => {
      if (/npm\b/.test(cmd)) return s.split("\n").filter((l) => !/^\s*(npm warn|npm notice|npm info|added|removed|changed)\b/.test(l)).join("\n")
      if (/git\b/.test(cmd)) return s.split("\n").filter((l) => !/warning: LF|hint:|^\s*$/.test(l)).join("\n")
      if (/ps\b/.test(cmd)) return s.split("\n").slice(0, 40).join("\n")
      return s
    }
    const compressPipeline = (raw: string, cmd: string) => {
      let s = ansiStrip(raw)
      s = commandFilter(s, cmd)
      s = jsonMinify(s)
      s = dropProgress(s)
      s = foldDupLines(s)
      s = smartTruncate(s)
      return s
    }
    const distill = (s: string) => {
      // 错误感知：保留【关键错误/状态行 + 结尾】，丢中间 bulk —— 模型仍能看错修错，成功率不受影响。
      const lines = s.split("\n")
      const KEY = /error|fail|exception|throw|warn|cannot|unable|✖|失败|错误|BUILD (SUCCESSFUL|FAILED)|Compil|\b\d+ error(s)?\b/i
      const keyLines = lines.filter((l) => KEY.test(l)).slice(0, 60).join("\n")
      const tail = s.slice(-FINAL_RESULT_KEEP)
      const kept = (keyLines.length + tail.length + 200)
      const verbose = s.length > FINAL_RESULT_KEEP * 6 || keyLines.length > 3
      const text = verbose
        ? `── 关键错误/状态行 ──\n${keyLines.trim() || "(无匹配)"}\n── 结尾 ──\n${tail.trim()}\n── end ──`
        : `── 最终结果 ──\n${tail.trim()}\n── end ──`
      return { text, kept: Math.min(kept, s.length) }
    }

    // rtk 式：逐命令语义压缩（有损但符合工具短结果格式）。无损靠调用方把 raw 落盘 KB 保回取。
    // 未识别的命令返回 null → 落回上面的通用 distill。
    const compactByCommand = (cmd: string, raw: string): string | null => {
      const is = (c: string) => new RegExp(`(^|[\s/:])${c}( ?|$)`, "i").test(cmd)
      const lines = ansiStrip(raw).split("\n").map((l) => l.replace(/[\r\n]+$/, "")).filter((l) => l.trim() !== "")
      const head = (a: string[], n = CAP_LIST) => { const h = a.slice(0, n); return h.join("\n") + (a.length > n ? `\n… +${a.length - n} more` : "") }
      // ---- ls / tree：目录列出 → 计数 + 前 N 项（仍像目录列表） ----
      if (is("ls") || is("tree")) {
        // 长格式(-l)：取权限串后的文件名(最后一个 token)；单列(-1)：整行即文件名
        const names = lines
          .filter((l) => !/^total \d/.test(l) && l !== "." && l !== "..")
          .map((l) => (/^[bcdlps-][rwx-]{9}/.test(l) ? (l.trim().split(/\s+/).pop() || l) : l))
        const dirs = names.filter((l) => l.endsWith("/"))
        const files = names.filter((l) => !l.endsWith("/"))
        const head0 = names.slice(0, CAP_LIST)
        return `${dirs.length} dir${dirs.length === 1 ? "" : "s"}, ${files.length} file${files.length === 1 ? "" : "s"}${head0.length ? ":\n" + head0.join("\n") : ""}${names.length > CAP_LIST ? `\n… +${names.length - CAP_LIST} more` : ""}`
      }
      // ---- git status：按状态分组（M/A/D/R/U/??）+ 数量，像 git 短格式 ----
      if (is("git") && /\bstatus\b/.test(cmd)) {
        const groups: Record<string, string[]> = {}
        for (const l of lines) {
          const m = /^([MADRCU?! ]{2}|[ MADRCU?]) (?:\s*)(.+)$/.exec(l)
          if (m) { const st = m[1].replace(/ /g, "") || "?"; (groups[st] ??= []).push(m[2]) }
        }
        const keys = Object.keys(groups)
        if (!keys.length) return raw.trim().slice(0, FINAL_RESULT_KEEP) // 干净 or 长格式 → 保尾
        return `git status (${lines.length - 1} 行)：\n` + keys.map((st) => `${st} (${groups[st].length}):\n${head(groups[st])}`).join("\n")
      }
      // ---- git log / show：保留每个 commit 的 hash+subject（oneline 语义），丢正文/stat 大段 ----
      if (is("git") && /\b(log|show)\b/.test(cmd)) {
        const commits: string[] = []
        for (let i = 0; i < lines.length; i++) {
          const m = /^commit ([0-9a-f]{7,40})/i.exec(lines[i])
          if (!m) continue
          let subject = ""
          for (let j = i + 1; j < lines.length && j <= i + 12; j++) {
            const t = lines[j].trim()
            if (!t) continue
            if (/^(Author|Date|Merge|Signed-off|Co-authored|commit )/i.test(t)) continue
            if (/^    /.test(lines[j])) { subject = t; break } // 正文缩进行 = subject
            subject = t; break
          }
          commits.push(`${m[1].slice(0, 12)} ${subject}`)
        }
        if (commits.length) {
          const headC = commits.slice(0, CAP_LIST)
          return `${commits.length} commits:\n${headC.join("\n")}${commits.length > CAP_LIST ? `\n… +${commits.length - CAP_LIST} more` : ""}`
        }
      }
      // ---- git diff / show：保留 per-file 行 + hunk 头 + 变更统计，丢 +/-正文 ----
      if (is("git") && /\b(diff|show)\b/.test(cmd)) {
        const files: string[] = []
        const hunks: string[] = []
        for (const l of lines) {
          const f = /^diff --git a\/(\S+)(?: b\/(\S+))?$/.exec(l)
          if (f) files.push(f[2] && f[2] !== f[1] ? `${f[1]} → ${f[2]}` : f[1])
          const h = /^@@ -(\d+(?:,\d+)?) \+(\d+(?:,\d+)?) @@(.*)$/.exec(l)
          if (h) hunks.push(`@@ -${h[1]} +${h[2]} @@${h[3] ? " " + h[3].trim() : ""}`)
        }
        const stat = lines.filter((l) => /\bfiles? changed\b|(\d+) insertion|(\d+) deletion/i.test(l)).slice(0, 3)
        const parts: string[] = []
        if (files.length) parts.push(`${files.length} files:\n${head(files, CAP_LIST)}`)
        if (hunks.length) parts.push(`${hunks.length} hunks:\n${hunks.slice(0, CAP_ERR).join("\n")}`)
        if (stat.length) parts.push(stat.join("\n"))
        if (parts.length) return parts.join("\n\n") + (raw.trim().endsWith(")") ? "" : "")
      }
      // ---- find：路径列表去重/封顶（rtk: cap + 文件类型） ----
      if (is("find")) {
        const paths = lines.filter((l) => !/Permission denied/.test(l) && !/^find:\s/.test(l))
        const dirs = paths.filter((l) => /\/$/.test(l))
        const files2 = paths.filter((l) => !/\/$/.test(l)).map((l) => l.trim().replace(/\/$/, ""))
        return `${dirs.length} dirs, ${files2.length} files:\n${head(files2.slice(0, CAP_LIST), CAP_LIST)}${files2.length > CAP_LIST ? `\n… +${files2.length - CAP_LIST} more paths` : ""}`
      }
      // ---- grep / rg：按文件分组 + 每文件封顶（rtk: group by file, cap, tee） ----
      if (is("grep") || is("rg")) {
        const groups: Record<string, string[]> = {}
        for (const l of lines) {
          const m = /^([^:]+):(\d+):(.*)$/.exec(l)
          const key = m ? m[1] : "(match)"
          ;(groups[key] ??= []).push(m ? `${m[2]}: ${m[3]}` : l)
        }
        const keys = Object.keys(groups)
        const parts: string[] = []
        for (const f of keys.slice(0, CAP_LIST)) {
          parts.push(`${f} (${groups[f].length}):\n` + groups[f].slice(0, 5).map((s) => "  " + s).join("\n") + (groups[f].length > 5 ? `\n  … +${groups[f].length - 5} rows` : ""))
        }
        if (keys.length > parts.length) parts.push(`… +${keys.length - parts.length} files`)
        return `grep (${keys.length} files, ${lines.length} matches):\n` + parts.join("\n")
      }
      // ---- 公共前缀辅助：wc 多文件剥共享目录（rtk: find_common_prefix） ----
      const commonPrefix = (arr: string[]) => {
        if (!arr.length) return ""
        let p = arr[0]
        for (const s of arr) { while (!s.startsWith(p)) p = p.slice(0, -1) }
        return p
      }
      // ---- wc：多文件剥公共路径前缀（如 a/b/c.txt a/b/d.txt → c.txt | d.txt），合计保留 ----
      if (is("wc")) {
        const rows = lines.map((l) => l.trim().split(/\s+/)).filter((r) => r.length >= 2)
        if (rows.length) {
          const total = rows.find((r) => r[r.length - 1] === "total")
          const filesR = rows.filter((r) => r[r.length - 1] !== "total")
          const paths = filesR.map((r) => r[r.length - 1])
          const prefix = paths.length > 1 ? commonPrefix(paths) : ""
          const shown = filesR.map((r) => `${r.slice(0, -1).join(" ")} ${r[r.length - 1].slice(prefix.length)}`).slice(0, CAP_LIST)
          return `wc (${filesR.length} 项，公共前缀"${prefix || (paths.length > 1 ? "(各行不同)" : "(单行)")}"已剥):\n` + shown.join("\n") + (total ? `\n${total.join(" ")}` : "") + (filesR.length > CAP_LIST ? `\n… +${filesR.length - CAP_LIST} more` : "")
        }
        return raw.trim().slice(0, FINAL_RESULT_KEEP)
      }
      // ---- du：保留各行 + total（若有多项则封顶） ----
      if (is("du")) {
        const total = lines.find((l) => /\ttotal$/i.test(l))
        const rows = lines.filter((l) => !/\ttotal$/i.test(l))
        return `du (${rows.length} 项):\n${head(rows, CAP_LIST)}${total ? `\ntotal: ${total.trim()}` : ""}`
      }
      // ---- df：表头 + 每挂载一行 + 封顶 ----
      if (is("df")) {
        const header = lines[0]
        const rows = lines.slice(1)
        return `df:\n${header}\n${head(rows, CAP_LIST)}${rows.length > CAP_LIST ? `\n… +${rows.length - CAP_LIST} more` : ""}`
      }
      // ---- ps：表头 + 关键列 + 封顶 ----
      if (is("ps")) {
        const hi = lines.findIndex((l) => /^\s*(PID|UID|USER)\s/.test(l))
        const header = hi >= 0 ? lines[hi] : lines[0]
        const rows = (hi >= 0 ? lines.slice(hi + 1) : lines.slice(1))
        return `ps:\n${header}\n${head(rows, CAP_LIST)}${rows.length > CAP_LIST ? `\n… +${rows.length - CAP_LIST} more` : ""}`
      }
      // ---- curl / wget：保留结尾状态/速率/错误 关键行 ----
      if (is("curl") || is("wget")) {
        const fin = lines.filter((l) => /HTTP\/|error|failed|cannot|resolve|(\d{2,})\s*K?B.*(s|%|done)|100%|Saved to|(unknown|total) size/i.test(l)).slice(-8)
        return (fin.length ? fin.join("\n") : raw.trim().slice(-FINAL_RESULT_KEEP))
      }
      // ---- 测试/构建/lint：报告状 —— 通过率 + 前 N 条失败，像测试报告 ----
      if (/(vitest|jest|mocha|phpunit|pytest|ctest|golangci)/.test(cmd) || (/test/.test(cmd) && /(pass|fail|✓|✗|✖|Tests:|Suites:)|failed/i.test(raw))) {
        const summary = lines.filter((l) => /(Tests:|test suites|passed|failed|✓ \d|✗ \d|Tests:|Suites:|passed\s+\d|failed\s+\d)/i.test(l)).slice(0, 4)
        const fails = lines.filter((l) => /(✖|✗|×|FAIL|failed|Error:|\berror[s]?\b)/i.test(l)).slice(0, CAP_ERR)
        const parts: string[] = []
        if (summary.length) parts.push(summary.join("\n"))
        if (fails.length) parts.push(`failures (${fails.length} shown):\n${fails.join("\n")}`)
        return parts.length ? parts.join("\n\n") : null
      }
      return null
    }

    // ———— 文件化知识库（倒排索引） ————
    const kbid = (raw: string) => { let h = 0; for (let i = 0; i < raw.length; i++) h = (h * 31 + raw.charCodeAt(i)) | 0; return `k${(h >>> 0).toString(16)}` }
    const tokenize = (s: string) => (s.toLowerCase().match(/[a-z0-9_]{2,}/g) || []).filter((w) => !["the","and","for","are","was","with","this","that","from","have","tsk"].includes(w))
    const kbsave = async (id: string, raw: string, title: string, command: string) => {
      try {
        const x = await kbfs(cwd); if (!x) return
        x.f.writeFileSync(`${x.d}/${id}.txt`, raw, "utf8")
        // metadata + 倒排索引
        const mdPath = `${x.d}/index.json5`
        let idx: any = { frags: {}, inv: {} }
        try { idx = JSON.parse(x.f.readFileSync(mdPath, "utf8")) } catch {}
        idx.frags[id] = { id, title: title.slice(0, 80), preview: ansiStrip(raw).replace(/\s+/g, " ").slice(0, KB_TITLE_PREVIEW), path: `${x.d}/${id}.txt`, len: raw.length, cmd: (command || "").slice(0, 120), ts: Date.now() }
        const words = tokenize(title + " " + ansiStrip(raw).slice(0, 2000))
        for (const w of new Set(words)) { (idx.inv[w] = idx.inv[w] || []); if (!idx.inv[w].includes(id)) idx.inv[w].push(id) }
        x.f.writeFileSync(mdPath, JSON.stringify(idx), "utf8")
        return id
      } catch { return undefined }
    }
    // —— B(context-mode 式)：大输出按空行切 section，每节 {title,content} 独立落盘+索引，
    //    使 tsk search(BM25)能精取单节；模型只看「章节地图」即可定向取回 ——
    const MAX_SECTIONS = 64
    const sectionize = (raw: string): { title: string; content: string }[] => {
      const blank = raw.split(/\n\s*\n/)
      const usable = blank.map((s) => s.trim()).filter((s) => s.length > 0)
      if (usable.length >= 2 && usable.length <= MAX_SECTIONS) {
        return usable.map((s, i) => ({ title: (s.split("\n")[0] || `Section ${i + 1}`).slice(0, 40), content: s }))
      }
      // 无空行结构 → 定长行组（前 60 行一组，余下封顶）
      const L = ansiStrip(raw).split("\n").map((l) => l.trim()).filter((l) => l)
      const chunks: { title: string; content: string }[] = []
      const per = 60
      for (let i = 0; i < L.length && chunks.length < MAX_SECTIONS; i += per) {
        const slice = L.slice(i, i + per)
        const a = i + 1, b = i + slice.length
        chunks.push({ title: `Lines ${a}-${b}`, content: slice.join("\n") })
      }
      return chunks
    }
    const indexSections = async (baseId: string, sections: { title: string; content: string }[], command: string): Promise<string[]> => {
      try {
        const x = await kbfs(cwd); if (!x) return []
        const mdPath = `${x.d}/index.json5`
        let idx: any = { frags: {}, inv: {} }
        try { idx = JSON.parse(x.f.readFileSync(mdPath, "utf8")) } catch {}
        const ids: string[] = []
        sections.forEach((se, i) => {
          const sid = `${baseId}s${i}`
          ids.push(sid)
          x.f.writeFileSync(`${x.d}/${sid}.txt`, se.content, "utf8")
          idx.frags[sid] = { id: sid, title: se.title, preview: ansiStrip(se.content).replace(/\s+/g, " ").slice(0, KB_TITLE_PREVIEW), path: `${x.d}/${sid}.txt`, len: se.content.length, cmd: (command || "").slice(0, 120), ts: Date.now() }
          for (const w of new Set(tokenize(se.title + " " + ansiStrip(se.content).slice(0, 800)))) { (idx.inv[w] = idx.inv[w] || []); if (!idx.inv[w].includes(sid)) idx.inv[w].push(sid) }
        })
        x.f.writeFileSync(mdPath, JSON.stringify(idx), "utf8")
        return ids
      } catch { return [] }
    }
    const kbsearch = async (q: string, top = 3) => {
      try {
        const x = await kbfs(cwd); if (!x) return []
        const idx = JSON.parse(x.f.readFileSync(`${x.d}/index.json5`, "utf8"))
        const words = tokenize(q)
        const scores = new Map<string, number>()
        for (const w of words) for (const id of (idx.inv[w] || [])) scores.set(id, (scores.get(id) || 0) + 1)
        return Array.from(scores.entries()).sort((a, b) => b[1] - a[1]).slice(0, top).map(([id]) => {
          const g = idx.frags[id]; return { id, title: g?.title, preview: g?.preview, len: g?.len }
        })
      } catch { return [] }
    }

    return {
      "tool.execute.before": async (input: any, output: any) => {
        // 默认 read 无损；TSK_READ_COMPRESS=1 时启用 read 骨架（受控复验「输入能否压」）
        const tool = input?.tool
        if (tool === "read") { void output; return } // 无损直通：不做输入改写（有损），沙箱执行走输出侧
        if (tool === "bash") {
          const cmd = String(output?.args?.command ?? output?.args?.cmd ?? "")
          // 永不启动模拟器/设备/上真机（用户强制，常驻）
          const NO_EMULATOR = /(^|[\s;])(emulator|qemu|avd)\b|deveco\s+run\b|hdc\s+(launch|start|install)\b|--device\b|start\s+(emulator|avd)\b/i
          if (NO_EMULATOR.test(cmd)) {
            output.args.command = `echo "[TSK 拦截：永不启动模拟器/设备/真机 (${cmd.slice(0, 80)}…)。已完成构建，不上设备运行。]"`
            await journal({ kind: "emulator-blocked", sessionID: input.sessionID, callID: input.callID, cmd: cmd.slice(0, 160) }).catch(() => {})
            return
          }
          if (/^\s*(curl|wget)\b/.test(cmd)) { output.args.command = `echo "[TSK 路由: 已拦截 ${cmd.slice(0, 60)} — WebFetch/网络被强制 deny，请改用本地数据或 ask]"` }
        }
      },

      "tool.execute.after": async (input: any, output: any) => {
        const tool = input?.tool
        // deveco 传给此 hook 的 input: {tool, sessionID, callID, args} —— args 才是工具参数
        const ia = input?.args ?? {}
        const cmd = typeof ia === "string" ? String(ia)
          : String(ia?.command ?? ia?.cmd ?? input?.input?.command ?? input?.input?.cmd ?? "")
        const src = typeof output?.output === "string" ? output.output : ""
        let lastSections: string = ""
        if (!SANDBOX_TOOLS.has(tool)) return
        // 避免套娃：读 TSK 自身 KB 原文(.tsk/kb/k*.txt / index.json5) → 直通真原文，绝不二次压缩。
        // 回读原文统一走 tsk search(BM25)；cat KB 文件是透明逃逸，不应被再压一次、导致取不回全文。
        if (/\.tsk[\\/]kb[\\/]/.test(cmd)) return
        // size gate：小输出(≤1600字符)直接放行不动，避免每个 bash 都被包一层「沙箱」噪声
        if (src.length <= FINAL_RESULT_KEEP * 2) return

        // rtk 式逐命令语义压缩：命中 → 用紧凑但符合工具格式的文本；未命中 → 通用管线。
        // 无损保证：只要做了紧凑，就一定把原文落盘 KB，模型可 read/cat 或 tsk search 取回完整内容。
        const compact = (await tskCompress(cmd, src)) ?? (compactByCommand(cmd, src) || null)
        let raw = src
        let final: { text: string; kept: number }
        if (compact !== null) {
          raw = src
          final = { text: compact, kept: compact.length }
        } else {
          // 建 sandbox-重跑的原始+蒸馏（read 类命令幂等 → 用重跑结果；否则用真实输出走管线）
          raw = src
          let distilledRaw = ""
          if (tool === "bash" && READONLY_CMD.test(cmd)) {
            const sb = await sandboxRun(cmd, cwd)
            if (!sb.over) { raw = sb.out; distilledRaw = compressPipeline(sb.out, cmd) }
          }
          if (!distilledRaw) distilledRaw = compressPipeline(src, cmd)
          final = distill(distilledRaw)
        }
        goldens.set(input?.callID ?? `c${Date.now()}`, { distilled: final.text, raw })

        // 大输出 → 文件化知识库；并把「取回原文的指针 + 检索方式」写进上下文（外置全文回读=显式检索）。
        // rtk 式命中 → 必落盘（无损）；否则沿用阈值 + 意图。
        let kbPointer = ""
        const wantKB = compact !== null
          || raw.length > KB_AUTO_THRESHOLD
          || (raw.length > KB_INTENT_THRESHOLD && /analy|summari|report|error|snapshot|parse|提取|分析|汇总/.test(cmd))
        if (wantKB) {
          const id = await kbsave(kbid(raw), raw, cmd, cmd)
          if (id) {
            kbPointer = "<" + (kbd(cwd).replace(/\\/g, "/") + "/" + id + ".txt") + ">"
            await journal({ kind: "kb-save", sessionID: input.sessionID, callID: input.callID, tool, cmd: cmd.slice(0, 160), rawBytes: raw.length, kbFile: `${kbd(cwd)}/${id}.txt`, kbIndex: `${kbd(cwd)}/index.json5` })
          }
        }
        // —— B(context-mode 式)：大输出分节落盘 + 章节地图 ——
        let sectionMap = ""
        if (wantKB && raw.length > KB_INTENT_THRESHOLD) {
          const secs = sectionize(raw)
          const secIds = await indexSections(kbid(raw), secs, cmd)
          if (secIds.length > 1) {
            const titles = secs.map((s) => s.title)
            const shown = titles.slice(0, 12)
            sectionMap = `\n§ 章节(${titles.length})：${shown.join(" | ")}${titles.length > 12 ? ` | …+${titles.length - 12}` : ""}\n§ 取某节：tsk search "<章节标题关键词>" 精确取回。`
          }
        }

        output.output =
          `[TSK 沙箱：该操作 {${tool}} 已在沙箱执行完成，原文 ${raw.length} 字符(≈${Math.ceil(raw.length / 4)} tok)。` +
          `中间过程未进入上下文，仅保留最终结果。\n` +
          (kbPointer
            ? `原始数据已落盘知识库(无损可回取)：${kbPointer}\n精确原文用 tsk search "关键词" 按需取回片段；cat/read 可得完整原文（直通）。\n`
            : `需要完整输出请再运行原命令查看。\n`) +
          `${final.text}${sectionMap}`
        // 章节数计入记账，便于报告核对分节收益
        lastSections = sectionMap ? (sectionMap.match(/§ 章节\((\d+)\)/)?.[1] ?? "") : ""
        await journal({
          kind: "sandbox-output", sessionID: input.sessionID, callID: input.callID, tool,
          command: cmd && cmd.length > 240 ? cmd.slice(0, 240) + "…" : cmd,
          original_chars: raw.length, original_tokens: Math.ceil(raw.length / 4),
          kept_chars: final.kept, saved_chars: raw.length - final.kept,
          saved_tokens: Math.ceil(raw.length / 4) - Math.ceil(final.kept / 4),
          sections: lastSections,
        })
      },


      "event": async ({ event }: any) => {
        const t = event?.type
        const sid = event?.properties?.sessionID ?? ""
        if (t === "session.event" && event?.properties?.message?.parts) {
          // 事件落盘（简化：每条 JSONL）
          try { const f = await fs(); if (f) { f.mkdirSync(`${cwd}/.tsk`, { recursive: true }); f.appendFileSync(eventsPath, JSON.stringify({ ts: Date.now(), sid, evt: JSON.stringify(event).slice(0, 800) }) + "\n") } } catch {}
          return
        }
        try {
          if (t === "session.created" || t === "session.compacted") {
            void kbsearch
            // 恢复时在 journal 记账（保持轻量；注入逻辑留给真实 compact 恢复）
            await journal({ kind: "session", event: "SessionStart", sessionID: sid, source: t === "session.compacted" ? "compact" : "startup", plugin_ver: "V7-readcompress" })
          }
        } catch {}
      },
    }
  },
}