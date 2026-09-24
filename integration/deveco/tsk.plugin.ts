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
const kbd = (cwd: string) => `${cwd}/.tsk/ctx`
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
        if (tool === "read") {
          await journal({ kind: "read-any", sessionID: input?.sessionID, file: String(output?.args?.filePath || "").slice(-36) }).catch(() => {})
          const args = output?.args; if (!args || typeof args.filePath !== "string") return
          if (args.offset != null && args.offset > 1) return // 分页=显式取回，不压
          const rk = (input?.sessionID || "") + ":" + args.filePath
          const readN = (readCount.get(rk) || 0) + 1; readCount.set(rk, readN)
          if (readN > 1) return // 回读同一文件 → 取全文（escape-hatch：不用再压）
          // 内联压缩（无子进程，同步）：读 .md 摘要改写到摘要文件，改写 filePath
          // 目的：避免 Bun.spawn 子进程的 async 续体在 deveco hook 里不完成。
          const f = await fs(); if (!f) return
          let text = ""
          try { text = f.readFileSync(args.filePath, "utf8") } catch { return }
          if (/\.(md|markdown)$/i.test(args.filePath) && text.length > 1500) {
            try {
const lines = text.split("\n"); let title = ""
              const sections: string[] = []; let curTitle = "", descGot = false, bulletGot = false
              let size = 0; const cap = 1200
              for (const l of lines) {
                const tr = l.trim()
                if (/^#\s/.test(l)) { if (!title) title = l.replace(/^#\s+/, "").trim(); continue }
                if (/^#{2,4}\s/.test(l)) { curTitle = l.replace(/^#{2,4}\s+/, "").trim(); sections.push("\n### " + curTitle); size += curTitle.length; descGot = false; bulletGot = false; continue }
                if (!curTitle || size > cap) continue
                if (!descGot && /[A-Za-z0-9一-龥]/.test(tr) && tr.length > 6 && !/^[-*•]\s|^\d+[.、]\s/.test(tr)) { sections.push("  ¤ " + tr.slice(0, 90)); size += 92; descGot = true; continue }
                if (!bulletGot && /^[-*•]\s|^\d+[.、]\s/.test(tr)) { sections.push("  · " + tr.replace(/^[-*•]\s+/, "").slice(0, 110)); size += 112; bulletGot = true; continue }
                if (size > cap) break
              }
              if (size > 120 && size < text.length * 0.5) {
                const h = (text.length * 31) >>> 0, sha = h.toString(16)
                const dir = `${cwd}/.tsk-inline` // 摘要写工作区内（read 权限覆盖该树；写工作区外会被 deveco deny permission）
                f.mkdirSync(dir, { recursive: true })
                const sumFile = `${dir}/${sha}.md`
                const abs = args.filePath.replace(/\\/g, "/")
                const body = "【这是 TSK 压缩后的摘要，不是原始文档】\n# " + (title || "") + "\n" + sections.join("\n") + "\n\n—— 摘要结束 ————\n原文完整文档（未压缩全文）位于: \n" + abs + "\n如需更完整/更详细内容，请 Read 上面这个文件路径。\n"
                f.writeFileSync(sumFile, body)
                await journal({ kind: "read-skeleton-back", sessionID: input.sessionID, file: args.filePath, sumFile, origChars: text.length, sumChars: size }).catch(() => {})
                args.filePath = sumFile
              }
            } catch {}
          }
          return
        }
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
        if (!SANDBOX_TOOLS.has(tool)) return
        // size gate：小输出(≤1600字符)直接放行不动，避免每个 bash 都被包一层「沙箱」噪声
        if (src.length <= FINAL_RESULT_KEEP * 2) return

        // 建 sandbox-重跑的原始+蒸馏（read 类命令幂等 → 用重跑结果；否则用真实输出走管线）
        let raw = src
        let distilled = ""
        if (tool === "bash" && READONLY_CMD.test(cmd)) {
          const sb = await sandboxRun(cmd, cwd)
          if (!sb.over) { raw = sb.out; distilled = compressPipeline(sb.out, cmd) }
        }
        if (!distilled) distilled = compressPipeline(src, cmd)

        const final = distill(distilled)
        goldens.set(input?.callID ?? `c${Date.now()}`, { distilled, raw })

        // 大输出 → 文件化知识库；并把「取回原文的指针 + 检索方式」写进上下文（外置全文回读=显式检索）
        let kbPointer = ""
        if (raw.length > KB_AUTO_THRESHOLD || (raw.length > KB_INTENT_THRESHOLD && /analy|summari|report|error|snapshot|parse|提取|分析|汇总/.test(cmd))) {
          const id = await kbsave(kbid(raw), raw, cmd, cmd)
          if (id) {
            kbPointer = "<" + (kbd(cwd).replace(/\\/g, "/") + "/" + id + ".txt") + ">"
            await journal({ kind: "kb-save", sessionID: input.sessionID, callID: input.callID, tool, cmd: cmd.slice(0, 160), rawBytes: raw.length, kbFile: `${kbd(cwd)}/${id}.txt`, kbIndex: `${kbd(cwd)}/index.json5` })
          }
        }

        output.output =
          `[TSK 沙箱：该操作 {${tool}} 已在沙箱执行完成，原文 ${raw.length} 字符(≈${Math.ceil(raw.length / 4)} tok)。` +
          `中间过程未进入上下文，仅保留最终结果。\n` +
          (kbPointer
            ? `原始数据已落盘知识库：${kbPointer}\n若需完整内容，请用 read/cat 直接读取该文件；检索索引(.tsk/ctx/index.json5)可按关键词 grep 定位相关片段。\n`
            : `需要完整输出请再运行原命令查看。\n`) +
          `${final.text}`
        await journal({
          kind: "sandbox-output", sessionID: input.sessionID, callID: input.callID, tool,
          command: cmd && cmd.length > 240 ? cmd.slice(0, 240) + "…" : cmd,
          original_chars: raw.length, original_tokens: Math.ceil(raw.length / 4),
          kept_chars: final.kept, saved_chars: raw.length - final.kept,
          saved_tokens: Math.ceil(raw.length / 4) - Math.ceil(final.kept / 4),
        })
      },

      "experimental.chat.messages.transform": async (input: any, output: any) => {
        // 诊断：dump 一条含 tool 的消息的 parts 结构（找对 L2 剪裁点）
        const dm = (output?.messages || []).find((m: any) => (m.parts || []).length > 1)
        // L2 上下文压缩：发给模型的 messages 可改写。保留最近 KEEP_RECENT 轮完整；
        // 更早的轮只留 text/reasoning（决策），删掉 verbose 工具结果 → 每轮重发的历史被压平。
        const KEEP_RECENT = 10
        const msgs = (output?.messages || []) as any[]
        if (true) return
        const keep = msgs.slice(-KEEP_RECENT)
        const old = msgs.slice(0, msgs.length - KEEP_RECENT)
        const oldCondensed: any[] = []
        let droppedChars = 0, droppedParts = 0
        for (const m of old) {
          const parts = (m.parts || []).filter((p: any) => p.type === "text" || p.type === "reasoning")
          const dropped = (m.parts || []).length - parts.length
          if (dropped > 0) {
            for (const p of (m.parts || [])) {
              if (p.type === "tool" && p.state?.output != null) droppedChars += String(p.state.output).length
            }
            droppedParts += dropped
          }
          if (parts.length) oldCondensed.push({ info: m.info, parts })
        }
        const notice = { info: { role: "user" }, parts: [{ type: "text", text: "[TSK 压缩: 较早轮次的工具输出已省略以控制上下文。若需某文件/日志原文，请按之前给出的 .tsk/ctx 指针 read/cat 取回。" }] }
        output.messages = [...oldCondensed, notice, ...keep]
        if (droppedChars > 0) await journal({
          kind: "l2-history-compress", sessionID: input?.sessionID ?? "",
          dropped_chars: droppedChars, dropped_parts: droppedParts,
          saved_tokens: Math.ceil(droppedChars / 4),
          msgs: msgs.length, kept: keep.length,
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