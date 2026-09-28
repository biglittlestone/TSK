# TSK × opencode — 怎样拿到 context-mode 的 token 压缩收益

> 结论先行：**opencode 平台已经实现了把收益拿到的硬机制（`tool.execute.after` 输出压缩）**。
> 缺的不是机制，是 ①一个真实运行时上的整文件 on/off 证明，②把「无损检索」从「指针+手 grep」变成「一等公民检索」。下面逐条对齐 context-mode 的三根受益立柱。

## context-mode 收益到底来自哪三件事（对照它的 BENCHMARK.md）

| 立柱 | context-mode | 能拿到收益的根本 | TSK-on-opencode 现状 |
|---|---|---|---|
| **M1 压缩发生在「工具响应层」** | `ctx_execute_file` 是 MCP 工具，只把 `console.log`/蒸馏回给模型 | 工具响应由插件掌控，原文 bytes 从不进上下文 —— 这是**静态保证的压缩比**（45KB log→155B≈99.7%），不依赖模型聪不聪明 | **已有**：`tool.execute.after` 重写 `output.output`，`distill()` 只留「关键错误/状态行+结尾」；超大落盘 `.tsk/ctx/k*.txt` + 倒排索引 + 指针。冒烟 11/11 绿 |
| **M2 整文件聚合任务** | 日志聚合 / CSV 统计 / 页面快照 / git-log 摘要 | 只有「备选方案是整贴原文」的任务，raw→distilled 比值才巨大、才有 delta | 需补**用对任务的证明**（见路线 ①）。我此前的 04 是提取类任务（grep 就够），所以看不出 delta —— 不是机制失效 |
| **M3 无损精确检索（付费的逃逸口）** | `ctx_index`+`ctx_search`（FTS5 BM25），精确 chunk 按需返回 | 全文不失真；只按取回量付费；代码/API 原样返回，模型不用重跑 | **半成品**：`kbsearch` 有实现但 `void kbsearch`（line 301）没暴露成可调工具；模型被告知「手 grep index.json5」（line 246）。且 deveco KB 布局是 `.tsk/ctx/`，和 claude-native `tsk search` 读的 `ext/`+`kb/` 不一致 |

## 为何这是「能拿到」（而非撞墙）

- M1 是核心，它绕开了 Claude-native 的两处死穴：`outputSummaries` 不喂模型、Read 骨架被逃逸口抵消。**opencode 的 `tool.execute.after` 直接改写 `output.output`**，压缩发生在模型收到之前 —— 与 context-mode 的 MCP 工具响应同构。
- 收益的算术是**工具侧保证的**：模型收到 `final.text`（≤ ~1KB），而不是原文。所以 per-tool 压缩比是确定性的 ~96%，不需要「模型主动改名」。

## 要真正"拿到"，落地三步

**① 证明：整文件任务上的真实 on/off（运行时依赖）**
- 用日志聚合 / 构建错误报告 / CSV 统计 / git-log 摘要这类「必整读」任务，plugin on/off 各跑一遍真实 deveco 会话，累加模型侧 input token。
- 现有插件已记 `sandbox-output`（orig/kept chars + saved_tokens）做台账，可接 deveco 已有的 `compare-report.mjs` 出 on/off 对比。
- **阻塞**：需在有 DevEco Code 运行时（IDE/CLI）的机器上跑（README 已注明"真实 DevEco 实测需在有运行时的机器上做"）。本机无该运行时，只验证到字节级蒸馏正确性（smoke）。

**② 关掉检索缺口：把 `kbsearch` 暴露成一等检索**
- 现状模型只能「手 grep index.json5」，不如 context-mode 的 `ctx_search`。
- 做：把 `kbsearch` 接到一个模型可真调的命令/工具（对齐 claude-native `tsk search` 语义），`<query>` → 返回标题+120 字预览+文件指针；并把 deveco KB 布局并到 `tsk search` 能读的地方（`ext/`/`kb/`），让同一套检索覆盖 Claude 与 opencode 两侧。

**③ 校准：可用收益的口径**
- 净收益 = 每工具 `raw−kept`（已 journal）− 检索时取回的字节。这正是 TSK `tsk report` 的账本口径，可扩到 opencode。
- 诚实边界：提取类任务（grep 可得）模型自己就产出小输出，收益趋零；整文件任务才有 ~96%。这不是 TSK 的 bug，是所有"摘要式压缩"的共性前提。

## 一句话

TSK 在 opencode 上已经把 context-mode 的**收益机制**装好了（M1 蒸馏输出 + M2/M3 部分），
差的只是 ①在有运行时的机器上跑一次整文件 on/off 把它量化、②把 M3 的 kbsearch 从「指针」升级成「可调工具」两点。做完这两点拿到 ≈ context-mode 的压缩比（per-tool 保证），并补上「付费逃逸口」让侧不漏纯净。