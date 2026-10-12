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
const FINAL_RESULT_KEEP = 800         // 输出压缩 size gate：≤800*2=1600B 放行（通用蒸馏已移除，仅做阈值）
const KB_INTENT_THRESHOLD = 5 * 1024  // 大输出且带 intent → 入知识库
const KB_AUTO_THRESHOLD = 100 * 1024  // 超过此自动入知识库
const CAP_LIST = 20                    // rtk 式：列表类命令保留前 N 项
const CAP_ERR = 20                     // 测试/构建：前 N 条失败
const CAP_WARN = 10
const KB_TITLE_PREVIEW = 120          // 检索只回标题 + 120 字符预览
const SANDBOX_TOOLS = new Set(["bash", "build_project", "arkts_check"])

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
    const readCount = new Map<string, number>() // file->read次数（首读压缩/回读还原全文 escape-hatch）

    const journal = async (row: any) => {
      try { const f = await fs(); if (!f) return; f.mkdirSync(`${cwd}/.tsk`, { recursive: true }); f.appendFileSync(journalPath, JSON.stringify({ ts: Date.now(), ...row }) + "\n") } catch {}
    }

    const tskCompress = async (cmd: string, raw: string): Promise<string | null> => {
      // 候选：PATH 里的 `tsk` → 用户目录固定安装路径（不依赖 PATH——IDE 重开的环境快照可能不含新加的 PATH）
      const home = process.env.USERPROFILE || process.env.HOME || ""
      const candidates = ["tsk", `${home}\\.tsk\\bin\\tsk.exe`, `${home}/.tsk/bin/tsk.exe`]
      for (const bin of candidates) {
        if (!bin) continue
        try {
          const proc = Bun.spawn([bin, "compress", cmd], { stdin: "pipe", stdout: "pipe", stderr: "pipe" })
          proc.stdin.write(raw); proc.stdin.end()
          const timeout = new Promise((_, rej) => { const t = setTimeout(() => { try { proc.kill() } catch {}; rej(new Error("compress-timeout-8s")) }, 8000) })
          const buf: Uint8Array = await Promise.race([new Response(proc.stdout).arrayBuffer(), timeout])
          const out = new TextDecoder().decode(buf).trim()
          if (out && out !== "{}") return out   // CLI 无命中打印 `{}`（hook passthrough 约定）→ 视为未匹配
        } catch {}
      }
      return null
    }
    const tskArgs = async (args: string[]) => {
      const home = process.env.USERPROFILE || process.env.HOME || ""
      const candidates = ["tsk", `${home}\\.tsk\\bin\\tsk.exe`, `${home}/.tsk/bin/tsk.exe`]
      for (const bin of candidates) {
        if (!bin) continue
        try {
          const proc = Bun.spawn([bin, ...args], { stdout: "pipe", stderr: "pipe" })
          const buf: Uint8Array = await new Response(proc.stdout).arrayBuffer()
          const s = new TextDecoder().decode(buf).trim()
          if (s) return s
        } catch {}
      }
      return ""
    }
    const ANSI = /\u001b\[[0-9;]*[A-Za-z]|[\u001b][()][0-9;]*[A-Za-z]|\[2K|\r/g
    const ansiStrip = (s: string) => s.replace(ANSI, "")

    // rtk 式：逐命令语义压缩（有损但符合工具短结果格式）。无损靠调用方把 raw 落盘 KB 保回取。
    // 未识别的命令返回 null → 调用方放行原文（通用蒸馏已移除，见 tool.execute.after）。
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
      "UserPromptSubmit": async (input: any, output: any) => {
        // 会话内 /tsk 命令：显示 TSK 累计收益（tsk report --all）。fail-open。
        const p = String(input?.prompt ?? input?.message ?? input?.text ?? "")
        if (!p.trim()) return
        if (!/^\s*\/tsk(\s*$|\s)/.test(p)) return
        try {
          const report = await tskArgs(["report", "--all"])
          if (report) {
            const note = `[TSK 累计收益]\n${report}`
            if (output && "prompt" in output) { output.prompt = `${p}\n\n${note}`; return }
            if (output && "updatedPrompt" in output) { output.updatedPrompt = `${p}\n\n${note}`; return }
            if (output && "message" in output) { output.message = `${p}\n\n${note}`; return }
          }
        } catch {}
      },
      "tool.execute.before": async (input: any, output: any) => {
        // 默认 read 无损；TSK_READ_COMPRESS=1 时启用 read 骨架（受控复验「输入能否压」）
        const tool = input?.tool
        if (tool === "read") { void output; return } // 无损直通：不做输入改写（有损），沙箱执行走输出侧
        if (tool === "bash") {
          const cmd = String(output?.args?.command ?? output?.args?.cmd ?? "")
          // 永不启动模拟器/设备/上真机（用户强制，常驻）
          const NO_EMULATOR = /(^|[\s;])(emulator|qemu|avd)\b|deveco\s+run\b|hdc\s+(launch|start|install)\b|--device\b|start\s+(emulator|avd)\b/i
          if (NO_EMULATOR.test(cmd)) {
            // 替换文本只留字母数字/中文——含括号/斜杠/引号会被 bash 解析成语法错误（实测嵌套 emulator 命令）
            const safe = cmd.slice(0, 60).replace(/[^A-Za-z0-9_一-龥]/g, "")
            output.args.command = `echo "[TSK 拦截：永不启动模拟器/设备/真机 ${safe}。已完成构建，不上设备运行。]"`
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

        // rtk 式逐命令语义压缩：命中 → 用紧凑但符合工具格式的文本；未命中 → 放行原文。
        // 通用蒸馏已移除（deveco-eval-net 实测）：丢信息压缩无理论依据、回读概率高，端到端不省。
        // 无损保证：只要做了紧凑，就一定把原文落盘 KB，模型可 read/cat 或 tsk search 取回完整内容。
        const compact = await tskCompress(cmd, src)
        if (compact === null) return // 命令不识别 → 放行原文，不做丢信息压缩
        const raw = src
        const final = { text: compact, kept: compact.length }

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

        // 包裹头极简（rtk 风格）：只告诉模型 3 件事——压缩幅度、全文位置、取回方式。
        // 不写"沙箱执行/中间过程"等内部细节（模型不需要知道实现），省固定开销。
        // tok 估算是 raw/4 的派生值，字节比已传达幅度，不再附 token 换算。
        const hdr = `[TSK {${tool}}: 压 ${raw.length}B→${final.kept}B]`
        const retr = kbPointer
          ? `\n全文: ${kbPointer} (cat 直通；tsk search "<关键词>" 精取)`
          : `\n需要完整输出请重跑原命令。`
        output.output = `${hdr}${retr}\n${final.text}${sectionMap}`
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