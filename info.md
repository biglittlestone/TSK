# TSK 实现手册（自包含规格书）

> **本文档自包含**：另一位实现者只拿这一份，即可无差别实现 TSK v1 的全部设计——
> 不需要本工程目录里的任何其他文件（架构文档、HTML 设计文档、spike 脚本均已内联或转写为规格）。
>
> TSK = **T**oken **S**aving **K**it。降 token 的 agent 插件：Rust 核心 + agent hooks。
> 三条压缩线：输入（skeleton / 沙箱分析）、输出（注入文风）、历史（快照 / 阈值建议）。
> 设计目标：(1) agent 插件形态 (2) 多档位压缩 (3) 输入输出双向 (4) 可插拔且用户无感知。
> 全部关键决策均经 9 项穿刺实验验证（数据内联于 §11–12）。

---

## 0. 八条铁律（任何 PR 违反即打回）

| # | 铁律 | 代码落点 |
|---|------|---------|
| 1 | 边界压缩永不回溯 | `policy::decide` 只返回"下一次调用"的 Plan；不存在以已发出内容为输入的 API |
| 2 | 同输入永远同字节 | compress/inject 全纯函数；CI fixture 逐字节比对 |
| 3 | 布局排序 + 多断点 | 注入按固定顺序拼装；serde struct 固定字段序 |
| 4 | 回溯重写必须批量+滞后+定时 | v1 无此功能 = 代码里没有这个路径 |
| 5 | compaction 是唯一合法全量重写 | snapshot 仅 PreCompact 事件触发 |
| 6 | cache 命中率是 CI 一等指标 | `tsk report --cache-hit`（M4） |
| 7 | 记账零跨 hook 依赖 | ledger.rs 纯追加写，无读取回路，不用 PostToolUse |
| 8 | 注入预算硬上限 | budget 最后 `while over { trim(P4→P3→P2) }`，P1 永不进 trim |

---

## 1. 仓库结构与 Cargo 清单

```
tsk/
├── Cargo.toml              # workspace
├── LICENSE                 # Apache-2.0 官方全文（发布前获取，见 §16）
├── NOTICE                  # 致谢文本（全文见 §16，逐字使用）
├── README.md               # 项目入口
├── core/                   # tsk-core：纯库，全部业务逻辑
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs          # pub mod + Error 体系
│       ├── config.rs       # 用户层 2 键配置 + 工程层常量
│       ├── events.rs       # hook 事件 serde 类型（§5）
│       ├── policy/
│       │   ├── mod.rs      #   decide(action) -> Plan
│       │   ├── task_type.rs#   debug / explore / refactor 识别
│       │   ├── routing.rs  #   skeleton vs sandbox 路由
│       │   └── escape.rs   #   逃逸口与复杂度检查
│       ├── compress/
│       │   ├── mod.rs      #   trait Compressor + tokenizer-count gate
│       │   ├── skeleton.rs #   head/tail/omission marker
│       │   ├── externalize.rs # >100KB 指针+摘要
│       │   └── deterministic.rs # 测试助手
│       ├── sandbox/
│       │   ├── mod.rs      #   run(script, input) -> stdout
│       │   ├── env.rs      #   黑名单过滤（§9）
│       │   └── limits.rs   #   网络/大小/超时
│       ├── inject/
│       │   ├── mod.rs      #   静态资产 include_str! 装配
│       │   ├── budget.rs   #   P1-P4 + 硬截断（§8.3）
│       │   └── clarity.rs  #   auto-clarity 判定
│       ├── ledger.rs       # 单向台账
│       ├── snapshot.rs     # PreCompact 快照
│       └── storage.rs      # .tsk/ 目录布局
└── cli/
    ├── Cargo.toml
    └── src/main.rs         # clap + stdio 编排，<500 行，fail-open 单点
```

**分工规则**：cli 不含业务逻辑；core 不依赖 clap。hook 单命令入口
`tsk hook <event>`——新增事件类型不需要改用户 settings.json。

**根 Cargo.toml**（逐字）：

```toml
[workspace]
resolver = "2"
members = ["core", "cli"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
sha1 = "0.10"
```

**core/Cargo.toml**：

```toml
[package]
name = "tsk-core"
description = "TSK core library: policy, compression, sandbox, injection, ledger, snapshot"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
serde_yaml = { workspace = true }
sha1 = { workspace = true }
```

**cli/Cargo.toml**：

```toml
[package]
name = "tsk-cli"
description = "TSK command-line entry: hook dispatch, init, report, doctor, exec"
version.workspace = true
edition.workspace = true
license.workspace = true

[[bin]]
name = "tsk"
path = "src/main.rs"

[dependencies]
tsk-core = { path = "../core" }
clap = { version = "4", features = ["derive"] }
serde_json = { workspace = true }

[dev-dependencies]
assert_cmd = "2"
```

**依赖纪律**：以上为全部依赖。加任何依赖需要书面理由。
不引入 tokio（hook 短命进程，`std::process::Command` 足够）；
不引入任何 LLM SDK / tokenizer（token 估算 = `ceil(len/4)`，与 CS-4 口径一致）。

---

## 2. 配置模型（三层）

### 2.1 用户层 — `~/.tsk/config.yaml` 全文（tsk init 生成，仅两键）

```yaml
enabled: true              # 一键全关（排查安全阀）
output_compression: off    # off | lite | full | ultra —— 唯一真正的用户决策
```

设计理由：唯一**非无损**项是输出压缩（改变回答文风），必须显式同意；
其余一切靠机制保证无损（逃逸口/回退链/外置可回读/fail-open），不交给用户调。

### 2.2 工程层 — 编译期常量（core/src/config.rs）

| 常量 | 值 | 依据 |
|------|-----|------|
| `SKELETON_HEAD` / `SKELETON_TAIL` | 5 / 3 行 | 穿刺 S2b、S4 |
| `ROUTE_THRESHOLD` | 50KB | CS-1 场景边界 |
| `EXTERNALIZE_THRESHOLD` | 100KB | 纯节省无副作用 |
| `SANDBOX_MAX_OUTPUT` | 64KB（stdout 超限→摘要+外置） | CS-1 |
| `SANDBOX_HARD_CAP` / `SANDBOX_TIMEOUT` | 100MB / 10s | 安全基线 |
| `INJECT_BUDGET` | 500 tok + 硬截断 | CS-4 |
| `INJECT_P1_MAX_CHARS` / `P2_N` / `P3_N` | 400 / 5（溢出降 3）/ 10 | CS-4 |
| `SNAPSHOT_MAX` | 2KB | 快照设计 |
| `COMPACT_ADVICE_THRESHOLD` | 180k input_tokens | S4 |
| `REINFORCEMENT_CHARS` | ~200 | S3+ |
| `RULESET_MIN_CHARS` | 3500 | S3（短注入被稀释） |
| 逃逸口 | offset/limit 恒开，不可配置 | S2b |
| 中文 | auto 检测 + 双语词典（下） | S5 |
| `ZH_KEYWORDS` | [修复, 新增, 重构, 测试, 文档, 性能, 构建, 集成, 风格] | S5 |

调试 TSK 本身：环境变量 `TSK_ADV_<CONST>` 临时覆盖（如 `TSK_ADV_ROUTE_THRESHOLD`），
不污染配置文件。

---

## 3. CLI 命令面

```
tsk init                    # 安装：写 <project>/.claude/settings.json hooks 段（已存在则合并，不覆盖其余键）+ 生成 ~/.tsk/config.yaml（零提问）
tsk hook <event>            # agent 调用入口；event ∈ {PreToolUse, PostToolUse, SessionStart, UserPromptSubmit, PreCompact}
tsk report [--session ID] [--cache-hit]   # 台账聚合：saved 总量、按工具分布、retrieval 次数、cache-hit
tsk doctor                  # 自检：config 存在且合法、~/.tsk 可写、settings.json 接线正确、tsk 在 PATH
tsk exec <script> [args]    # 沙箱执行（§9 规格全量适用）；执行完自行追加 ledger kind:sandbox 行
```

事件分工：PreToolUse 是唯一改写点；PostToolUse v1 恒 passthrough（S2 证明无法改写
tool_result，收它是为了 M0 协议骨架完整 + 留 v1.5 观测口）；其余三类只做注入与快照。

刻意不做：`tsk query/search`（CS-2：原生 Grep 完胜自建索引）、`tsk daemon`（无驻留需求）。

**tsk init 写入的 hooks 段**（逐字）：

```json
{
  "hooks": {
    "PreToolUse": [
      { "matcher": "Read", "hooks": [ { "type": "command", "command": "tsk hook PreToolUse" } ] }
    ],
    "PostToolUse": [
      { "matcher": "Read", "hooks": [ { "type": "command", "command": "tsk hook PostToolUse" } ] }
    ],
    "SessionStart": [
      { "hooks": [ { "type": "command", "command": "tsk hook SessionStart" } ] }
    ],
    "UserPromptSubmit": [
      { "hooks": [ { "type": "command", "command": "tsk hook UserPromptSubmit" } ] }
    ],
    "PreCompact": [
      { "hooks": [ { "type": "command", "command": "tsk hook PreCompact" } ] }
    ]
  }
}
```

PreToolUse 只 matcher Read（v1 只改写 Read；Bash 复杂度约束见 §5.3，Bash 改写 v1 不做）。
PostToolUse matcher 同 Read，行为恒 passthrough + 记日志（见上文事件分工）。
init 对已存在的 settings.json 做 JSON 合并——保留用户已有键，只添/更新 TSK 的 hooks 段；损坏或非 JSON 则报错退出而非覆盖（doctor 可检出）。

---

## 4. 五层架构 → 模块映射

```
L1 Agent Hooks     events.rs（协议）+ cli（stdio 编排）
L2 Policy          policy/：路由、逃逸口、任务类型、auto-clarity 判定
L3a Skeleton       compress/skeleton.rs（≤50KB 精读路径）
L3b Sandbox        sandbox/（统计/大文件路径，stdout 结论进 context）
L3c 注入/外置/快照  inject/ + compress/externalize.rs + snapshot.rs
L4 Storage+记账    storage.rs + ledger.rs
```

存储布局：

```
~/.tsk/
├── config.yaml
└── <session_id>/                  # 会话数据（机器私有）
    ├── ledger.jsonl               # 单向台账
    ├── resume-snapshot.json       # PreCompact 快照, <2KB
    ├── events.log                 # grep 友好事件日志（§6.5）
    ├── logs/                      # hook stderr 日志，按天滚动
    └── ext/<sha1>.txt             # 外置大输出

<project>/.tsk/                    # 项目工件（可 gitignore）
└── skeletons/<hash>-<name>.skeleton.txt
```

骨架是项目工件（确定性路径派生），会话数据机器私有——两者分开。

---

## 5. Hook 协议（Claude Code 2.1.117 实测口径）

### 5.1 stdin JSON（PreToolUse 实测确认的全部字段）

```json
{
  "session_id": "ccfb1a35-fa64-489e-acde-3ed3aa5871fc",
  "transcript_path": "C:\\Users\\...\\projects\\<slug>\\<session>.jsonl",
  "cwd": "D:\\project",
  "permission_mode": "default",
  "hook_event_name": "PreToolUse",
  "tool_name": "Read",
  "tool_input": { "file_path": "D:\\project\\bigfile.txt", "offset": 10, "limit": 5 },
  "tool_use_id": "chatcmpl-tool-391b8976efb1468097e7da3d5b72821f"
}
```

- `tool_use_id` 每次工具调用唯一（CS-3 实测确认）——ledger 记账用它。
- SessionStart 额外字段：`"source": "startup" | "clear" | "compact" | "resume"`。
- UserPromptSubmit 额外字段：`"prompt": "<用户消息文本>"`。
- PreCompact 字段（文档口径，S4 未实际触发过，M4 验收时实测核对）：`"trigger": "manual" | "auto"`、`"custom_instructions"`。

### 5.2 stdout 输出协议（按事件）

| 事件 | 成功输出 | 失败输出（恒定） |
|------|---------|----------------|
| PreToolUse | `{"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":{...改写后的完整 tool_input...}}}` | `{}` |
| PostToolUse | `{}`（v1 恒 passthrough，见 §3 事件分工；记 stderr 日志即返回） | `{}` |
| SessionStart | `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"..."}}` | `{}` |
| UserPromptSubmit | `{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"..."}}` | `{}` |
| PreCompact | `{"hookSpecificOutput":{"hookEventName":"PreCompact","additionalContext":"...锚点保护指令..."}}` | `{}` |

**三条纪律**：
1. stdout **永远最后写、永远完整合法 JSON**——半截 JSON 比不输出更糟。
2. 错误细节走 stderr + `~/.tsk/<session>/logs/`，stdout 只带决策结果。
3. core 内部 `Result` 传播不吞错；cli main 是唯一 catch 点（§10）。

### 5.3 平台机制约束（穿刺实测，实现时不可违反）

| 约束 | 来源 | 实现含义 |
|------|------|---------|
| 权限按**原命令**判，改写发生在判权限之后 | S1 | 免弹窗成立的前提 |
| 改写后命令复杂度不得超过原命令 | S1 后续发现 | **禁 node -e、禁管道、禁多语句**——否则触发二次权限检查弹窗 |
| PostToolUse **无法改写 tool_result**（4 种模式全失败） | S2 | 一切改写必须在 PreToolUse；v1 的 PostToolUse 恒 passthrough（仅协议占位 + v1.5 观测口，§3） |
| PreToolUse 改写 `file_path` 可行 | S2b | 输入压缩主路径 |
| offset/limit 重读会与改写形成无限循环 | S2b | 逃逸口恒开 |

---

## 6. 数据格式规格（全部确定性生成，铁律 2）

### 6.1 skeleton 文件格式

```
[TSK SKELETON of <原文件相对路径> | <N> lines total | head 5 + tail 3 | TSK v1]
     1	<第 1 行原文>
     2	<第 2 行原文>
     3	<第 3 行原文>
     4	<第 4 行原文>
     5	<第 5 行原文>
[TSK omitted lines 6..<N-3>. Full file: Read <原路径> with offset/limit, or Grep <pattern> <原路径>]
 <N-2>	<倒数第 3 行原文>
 <N-1>	<倒数第 2 行原文>
 <N>	<最后一行原文>
```

- 行号右对齐 6 列 + TAB + 原文（上面示意用空格，实现取 TAB）。
- 文件 < 8 行时不压缩（head+tail 会重叠/全量）。
- 非 UTF-8 / 二进制文件：不压缩，passthrough（逃逸规则）。
- 路径派生纯函数：`derive_skeleton_path(orig) = <project>/.tsk/skeletons/<sha1(orig 绝对路径规范化)>-<basename>.skeleton.txt`。
  同一原路径永远同一骨架路径（召回锚完整性）。
- 锚点格式 `[TSK SKELETON of ...]` 已被 S4 验证：模型能正确使用锚点、不幻觉文件内容、能说出"我没读过原文件，但能从锚点检索到"。

### 6.2 外置指针格式（>EXTERNALIZE_THRESHOLD 的文件走 Read 时）

```
[TSK EXTERNALIZED output of Read | original <size> bytes | full: ~/.tsk/<session>/ext/<sha1>.txt]
[TSK excerpt: head 1KB follows; retrieve full content via Read on the path above]
<前 1KB 原文>
```

**范围限定（重要，源自 S2 的平台限制）**：v1 的外置只作用于 TSK 能控制进入
context 的工件——Read 的文件（路径改写为指针文件）和沙箱 stdout。
Bash 等工具的**结果**无法事后改写（PostToolUse 不可改写 tool_result），
v1 不处理，记为已知限制。

### 6.3 ledger.jsonl 行格式（单向追加，铁律 7）

```json
{"ts":1789473338663,"session":"ccfb1a35","tool":"Read","tu_id":"chatcmpl-tool-391b...","kind":"rewrite","strategy":"skeleton","orig_bytes":143200,"new_bytes":647,"saved":142553}
{"ts":...,"kind":"retrieval","tool":"Read","reason":"escape_hatch_offset_limit","orig_bytes":143200}
{"ts":...,"kind":"externalize","tool":"Read","orig_bytes":104857,"new_bytes":1200}
{"ts":...,"kind":"inject","source":"session_start_ruleset","bytes":3512}
{"ts":...,"kind":"sandbox","tool":"Bash","orig_bytes":52329,"new_bytes":214,"saved":52115}
```

字段顺序固定（serde struct 序）。写失败记 stderr 日志，不阻断。
已知误差 ~5%（改写后调用未真正执行；权限判定在改写前所以少见）——已接受。

### 6.4 resume-snapshot.json（<2KB 硬顶）

```json
{"v":1,"session":"...","role":"≤400 字符角色描述","decisions":["最近决策，≤5 条"],"skills":["去重技能名，≤10 条"],"intent":"当前意图一句话"}
```

超 2KB 时按 P4→P3→P2 截（§8.3 同序）。

### 6.5 events.log（grep 友好，CS-2 的存储侧结论）

```
[2026-09-15T12:01:47Z] decision: use Rust core engine, not Go, for performance
[2026-09-15T12:02:03Z] error: tiktoken BPE download blocked by enterprise SSL proxy
```

一行一事、`类别: 内容` 格式——原生 Grep 一次命中（CS-2 实测 8/8 vs FTS5 2/8，
且 token 便宜 2.4 倍）。**禁止**为召回引入任何索引（SQLite/FTS5/向量）。

**写入者与事件来源（v1 范围，防实现者扩大化）**：
- TSK 自己写：PreCompact 时（§7.4）从 transcript 提取的 role/decision/skill/intent
  快照同时落一份到这里（快照 JSON 给注入，events.log 给 Grep 召回）；
  逃逸口触发、沙箱执行、外置触发也各记一行（类别 retrieval/sandbox/externalize）。
- 类别枚举（v1）：`decision / skill / role / intent / retrieval / sandbox / externalize / compact_advice`。
- 模型生成的自由文本事件（如 assistant 的错误报告）v1 **不采集**——hook 只在自己的
  决策点写，不做 transcript 全量镜像（那会把 events.log 变成第二个 context）。

---

## 7. 逐事件数据流

### 7.1 PreToolUse(Read) — 输入压缩主路径

```
stdin
 → enabled? 否 → passthrough
 → tool_input 有 offset 或 limit? → 是 → passthrough + ledger{kind:retrieval, reason:escape_hatch}
 → 文件不存在 / <8 行 / 非 UTF-8? → passthrough
 → 路由（policy/routing.rs）:
     文件 > ROUTE_THRESHOLD(50KB) 且任务匹配"统计/聚合"模式 → Plan::Sandbox
     文件 > EXTERNALIZE_THRESHOLD(100KB) 非统计 → Plan::Externalize
     否则 → Plan::Skeleton
 → Plan::Sandbox:  沙箱跑分析脚本 → stdout 摘要 → 失败回退 Plan::Skeleton
 → Plan::Skeleton: 生成/复用骨架文件 → updatedInput.file_path = 骨架路径
 → Plan::Externalize: 写 ext/<sha1>.txt + 指针文件 → updatedInput.file_path = 指针文件
 → ledger 追加对应 kind 行（stdout 之前）
 → stdout hookSpecificOutput.updatedInput
```

沙箱任务判定规则（"统计/聚合"模式识别）**未定**——M2 开工前先跑前置穿刺 P-1
（§14）：收集 20 个真实任务样本标注"精读意图 vs 统计意图"，测自动判定准确率。
判定规则落地前，路由退化为纯大小阈值（>50KB 一律 Sandbox 不成立——**保守默认：
只有显式匹配才走 Sandbox，其余全走 Skeleton**，错判方向必须是"多读原文件"而非"丢语义"）。

**Plan::Sandbox 的执行机制（v1 范围限定，防实现者自行脑补）**：
hook 判定 Plan::Sandbox 后，**不是**在 hook 进程里现造分析脚本。v1 机制：
1. hook 在 `<project>/.tsk/sandbox/` 下查找**已存在的**分析脚本（用户或 agent 事先放置，
   如 `analyze_<ext>.<sh|js|py>`）；找到 → 经 §9 沙箱基线执行，stdout 摘要进 context。
2. 没有现成脚本 → **直接回退 Plan::Skeleton**，不猜测任务意图、不生成代码。
3. "hook 内自动生成分析脚本并执行"是 context-mode 的做法，TSK v1 **明确不做**——
   hook 有 5s 预算（P-3），且自动生成代码执行的错误面远大于收益。
4. agent 主动调 `tsk exec <script>`（命令面 §3）始终可用——CS-1 的 B 组就是这个形态：
   agent 用 Bash 调 `tsk exec scripts/analyze_log.js`，只 print 6 行统计摘要。

### 7.2 SessionStart

```
 → source ∈ {startup, clear, resume, compact}
 → output_compression != off → additionalContext += ruleset 文案（§8.1，≥3500 chars）
 → source == resume 且快照存在 → additionalContext += snapshot 组装注入（§8.3，≤500 tok）
```

### 7.3 UserPromptSubmit

```
 → output_compression != off 且非首条 → additionalContext += 每轮强化（§8.2）
 → 上一轮 input_tokens > 180k → additionalContext += 一行 /compact 建议注入
```

**auto-clarity 的实现形态（P-2 穿刺前的保守落地）**：v1 的 auto-clarity **不是**
hook 在流中检测危险操作再动态改注入——hook 看不到 assistant 输出，做不了输出后检测。
v1 机制 = **预防性声明进注入文案**（§8.1 的 AUTO-CLARITY EXCEPTION 段 +
§8.2 强化里的 "Auto-clarity for safety content still applies"），让模型在生成时
自判：回复涉及安全/不可逆内容（DROP TABLE、rm -rf、force-push、凭证、合规）或
用户表达困惑时，该轮自动用正常清晰文风。触发正确性由 P-2 穿刺验证
（mock DROP TABLE / rm -rf 场景，§14）；不通过则考虑 UserPromptSubmit 侧
对 prompt 内容做关键词检测、命中时本轮强化文案换成 clarity 版（M3 里决策）。
inject/clarity.rs v1 只承载这个声明文案的选择逻辑（输出压缩开 → 文案含
AUTO-CLARITY 段；关 → 无），不做输出检测。

**180k 建议的数据源**：hook 不可能直接拿到 API usage——注入读的是
`transcript_path` 指向的会话 JSONL，从最后一条 assistant 消息的 usage 字段
（`message.usage.input_tokens` + cache_read_input_tokens）取上一轮实际值。
读不到/字段缺失 → 不建议（fail-open）。建议文案一行，如：
`Context is above 180k tokens; consider running /compact.`（不重复注入——
用 `~/.tsk/<session>/events.log` 里是否已有 compact_advice 行做幂等，
这是 v1 唯一一处"读自己的追加日志"，只在 UserPromptSubmit 路径，无并发面）。

### 7.4 PreCompact

```
 → snapshot::build(transcript_path)：提取 role/decisions/skills/intent
    （从 transcript JSONL 里取最近 assistant 文本与决策句）
 → 写 resume-snapshot.json（<2KB）
 → additionalContext = 锚点保护指令：
   "The context window is being compacted. Preserve verbatim every line starting
    with '[TSK SKELETON of' and every '[TSK ' marker line. These are retrieval
    anchors; do not summarize or drop them."
```

**snapshot 提取规则（P1-P4 逐项，确定性实现——铁律 2）**：
- **role（P1）**：会话第一条 user 消息里的任务描述句，截前 400 chars。
  取不到（首条过短）→ 用 transcript 里出现频次最高的文件路径集合拼一句
  "Working on: <paths>"；再取不到 → 空串（组装时 P1 空则注入只剩 P2-P4）。
- **decisions（P2）**：assistant 文本里匹配决策句式的行——正则
  `(?i)^(?:decision|决定)[:\s]` 或含 "we will use / chosen / 采用 / 选定" 的
  陈述句，按时间序去重，取最近 5 条，每条截 200 chars。
- **skills（P3）**：transcript 里出现过的工具名 + 文件扩展名推断的技能词
  （rust/javascript/python/build...），Set 去重，最近 10 条。
- **intent（P4）**：最后一条 user 消息截前 200 chars。
- 全程**只读 transcript、不调模型**；任何解析失败 → 对应项空串，hook 继续（fail-open）。

**cache-hit 指标的数据源（铁律 6，M4）**：`tsk report --cache-hit` 从
transcript JSONL 的 `message.usage` 聚合——`cache_read_input_tokens / input_tokens`
按轮平均；CLI 现场读 transcript 计算（report 不常驻，无需 hook 期采集）。
CI 阈值告警：对比有 TSK / 无 TSK 两条基线跑相同 fixture，cache-hit 比值低于
基线 -10 个百分点即 fail（数字在 M4 校准，先落机制）。

---

## 8. 注入资产（静态文件，include_str! 编译进二进制）

### 8.1 SessionStart ruleset（≥3500 chars，S3 实测阈值）

结构要求（S3 验证的结构）：硬规则 + 违规契约 + 正反例 + 自检指令。
以下为 TSK v1 正式文案（full 档，英文——S3 实验口径；lite/ultra 为其子集/超集，M3 产出）：

```text
TSK OUTPUT MODE ACTIVE (level: full). This mode reduces token usage. It does NOT
change technical correctness. Follow ALL rules below for every reply in this
session.

HARD RULES:
R1. No pleasantries, greetings, apologies, filler, or "Great question" openers.
    First word of the reply is technical content.
R2. Prefer sentence fragments over full sentences. Telegraphic style is correct
    here. Subjects and articles may be dropped when meaning is unambiguous.
R3. Code, commands, file paths, identifiers, error messages, and data stay
    VERBATIM. Never abbreviate, paraphrase, or truncate them.
R4. Prose explanations: max 2 sentences per reply unless the user explicitly
    asks for elaboration. If code can answer, code answers.
R5. No meta-commentary about your style, this mode, or token savings unless asked.
R6. No recap of previous turns. No restating the question. No "In summary".
R7. Lists and tables are preferred over paragraphs.
R8. Before emitting, self-check: does the reply contain any greeting, filler,
    recap, or >2 sentences of prose? If yes, cut it, then emit.

VIOLATION CONTRACT: A reply that violates R1-R7 while this mode is active is a
bug. Correct output looks like the POSITIVE examples, never like the NEGATIVE
examples.

POSITIVE example (user: "why is my loop off by one?"):
  Off-by-one: `i <= arr.length` should be `i < arr.length`. Fix: line 12.

POSITIVE example (user: "explain closures"):
  Closure = fn capturing outer scope vars after outer fn returned.
  ```js
  function counter() { let n = 0; return () => ++n; }
  ```
  `n` lives in `counter`'s scope; returned fn keeps access. That capture is the closure.

NEGATIVE example (wrong, do not do this):
  "Great question! Closures are a fascinating topic. Let me walk you through
  them step by step. First of all, it's important to understand..." — banned:
  opener + filler + tutoring voice.

AUTO-CLARITY EXCEPTION: Security warnings, destructive/irreversible operations
(DROP TABLE, rm -rf, force-push, credentials, legal/compliance), and user
confusion signals ("I don't understand") switch you to normal clear prose for
that reply, then back to full mode. Never compress safety-critical content.

SCOPE: This mode governs prose style only. It never governs code content,
commit messages, PR descriptions, or documentation files — those stay in
normal professional style. When in doubt, clarity wins over brevity.
```

长度校验：实现时用 `RULESET_MIN_CHARS=3500` 断言，不足则扩例句。
文案是 TSK 原创资产（NOTICE 声明口径，§16）。

### 8.2 每轮强化（~200 chars，S3+ 验证：长注入之上再 -19%）

```text
TSK OUTPUT MODE ACTIVE (full). Enforce this reply: no pleasantries, no filler,
no preamble, no recap. Prefer fragments. Code, commands, paths, errors stay
verbatim. Max 2 sentences of prose unless code is the answer. Auto-clarity for
safety content still applies.
```

### 8.3 snapshot → resume 注入组装算法（含 CS-4 硬截断补丁）

```
est(s) = ceil(len(s) / 4)                    # 与 CS-4 同口径
BUDGET = 500

assemble(snapshot):
    p1  = snapshot.role[0..400]              # P1 永不截断
    dec = snapshot.decisions[-5..]           # P2
    skl = dedup(snapshot.skills)[-10..]      # P3
    p4  = snapshot.intent                    # P4

    # context-mode 原版逻辑（CS-4 移植）：5→3 回退
    if est(p1)+est(dec)+est(skl)+est(p4) > BUDGET:
        dec = snapshot.decisions[-3..]

    # TSK 硬截断补丁（原版缺失，病态输入实测溢出 84 tok）
    while est(p1)+est(dec)+est(skl)+est(p4) > BUDGET:
        dropped = false
        if p4非空:        p4 = ""; dropped = true        # 先砍 P4
        elif len(skl)>1:  skl = skl[1..]; dropped = true # 再砍 P3（每次减 1）
        elif len(dec)>1:  dec = dec[1..]; dropped = true # 再砍 P2
        if not dropped: break               # 只剩 P1（400 chars ≈ 100 tok，必 < 500）

    return "ROLE: {p1}\nDECISIONS: {dec | }\nSKILLS: {skl | }\nINTENT: {p4}"
```

P1 单独最坏 ~100 tok，恒 < 500——循环必然终止。P2-P4 全空时输出仍含 P1。

---

## 9. 沙箱安全基线（tsk exec / Plan::Sandbox 全量适用）

1. **环境变量黑名单**（env.rs，清除而非继承）。原则：任何能导致解释器启动时
   执行任意代码、加载额外库、劫持模块路径的变量都清除。清单：

```
Shell/启动:   BASH_ENV, ENV, GITHUB_ENV, PROMPT_COMMAND, ZDOTDIR, IFS,
              GIT_CONFIG, GIT_CONFIG_COUNT, GIT_CONFIG_KEY_*, GIT_CONFIG_VALUE_*
JS/Node:      NODE_OPTIONS, NODE_PATH, NODE_EXTRA_CA_CERTS, ELECTRON_RUN_AS_NODE
Python:       PYTHONSTARTUP, PYTHONINSPECT, PYTHONPATH, PYTHONHOME
Perl/Ruby:    PERL5OPT, PERL5LIB, PERLLIB, RUBYOPT, RUBYLIB
JVM:          JAVA_TOOL_OPTIONS, _JAVA_OPTIONS, JDK_JAVA_OPTIONS
.NET:         CORECLR_PROFILER, CORECLR_PROFILER_PATH, CORECLR_ENABLE_PROFILING,
              COMPlus_Profiler*, DOTNET_STARTUP_HOOKS, DOTNET_ADDITIONAL_DEPS,
              DOTNET_SHARED_STORE, DOTNET_ENVIRONMENT
动态加载:     LD_PRELOAD, LD_LIBRARY_PATH, LD_AUDIT,
              DYLD_INSERT_LIBRARIES, DYLD_LIBRARY_PATH, DYLD_FRAMEWORK_PATH
Windows:      PSModulePath
代理(网络禁用的一部分): HTTP_PROXY, HTTPS_PROXY, ALL_PROXY, NO_PROXY,
              http_proxy, https_proxy, all_proxy
```

   `GIT_CONFIG_KEY_*` / `GIT_CONFIG_VALUE_*` / `COMPlus_Profiler*` 按前缀匹配清除。

   白名单转发：`PATH, HOME, LANG, LC_ALL(原值), TEMP/TMP, SYSTEMROOT, SYSTEMDRIVE, COMSPEC, USERNAME`。
   （**不新增也不修改 LC_ALL**——S5 铁律：代码库任何位置不得出现 `LC_ALL=C`。）

2. **网络 deny**：清除代理变量 + 不给任何凭证；进程级网络隔离（seccomp/Job 对象）v1 不做，文档声明"沙箱防注入不防外连"，记遗留。

3. **硬顶**：输入文件 >100MB 拒绝；stdout >64KB 截断为摘要 + 全文外置 ext/。

4. **超时**：10s；kill 整个进程树（Unix: `killpg`；Windows: `taskkill /T /F`）。

5. **回退链**：sandbox 失败 → Skeleton → 原文件 passthrough，每级 fail-open。

---

## 10. 失败模型（fail-open 统一定义）

```rust
fn main() {
    let out = run_cli(args, stdin).unwrap_or_else(|e| {
        eprintln!("[tsk] fail-open: {e}");   // stderr + 本地日志，不进 agent context
        CliOutput::passthrough()              // stdout: "{}"  exit 0
    });
    out.emit();                               // stdout 永远最后、永远完整 JSON
}
```

- 模块内**不 catch**：core 用 `Result` 一路传播，决策集中到 cli 单点，避免"部分执行"。
- fail-open 语义：hook 崩溃 = 本轮无 TSK，会话照常。宁可少省 token，绝不阻塞。

---

## 11. 穿刺结论全表（实现时的"为什么"索引）

### 11.1 平台机制类（S 系列）

| # | 结论（实测数据） | 实现影响 |
|---|----------------|---------|
| S1 | 权限按原命令判、改写在后（echo ORIGINAL→REWRITTEN 免弹窗实测通过）；但复杂度超原命令触发二次权限检查 | §5.3 复杂度约束 |
| S2 | PostToolUse 4 模式（block/additionalContext/updatedToolResponse/hookSpecificBlock）全部无法改写 tool_result | 一切改写在 PreToolUse；v1 不用 PostToolUse |
| S2b | PreToolUse 改写 file_path 可行；带 offset/limit 重读会无限循环（实测多耗 4 轮 + 2x 成本） | 逃逸口恒开 |
| S3 | 注入长度对照：0→505 tok / ~330 chars→394 (-22%) / ~1500→275 (-46%) / **~3500→179 (-65%)**；input 增量仅 +2.8% | RULESET_MIN_CHARS=3500；ruleset 结构=规则+正反例+自检 |
| S3+ | 5 轮 A/B（仅长注入 vs +每轮强化）：1225 vs 990 out_tok（**-19%**）；两组 5 轮内都无漂移；B 组首轮 +36 tok 一次性开销 | 每轮强化默认开（输出压缩开时）；20+ 轮未测=已知风险 |
| S4 | 16×200 行文件跑到 200,082 input_tokens：auto-compaction **未触发**（API retry 代替）、PreCompact hook **未调用**、但模型正确保留全部 16 个 `[TSK SKELETON of file_NN.txt]` 锚点、无内容幻觉 | 不依赖 auto-compaction；180k 建议线；PreCompact 防御性保留；锚点格式已验证 |
| S5 | 英文关键词 filter 丢 40% 纯中文 commit（3 条只留 1 条）；rtk `git.rs` 强制 `LC_ALL=C` 加剧 | 双语词典；全代码库禁 LC_ALL 设置 |

### 11.2 context-mode 融合类（CS 系列）

| # | 结论（实测数据） | 实现影响 |
|---|----------------|---------|
| CS-1 | 同批 52KB/500 行 nginx 日志分析：直接 Read 143,200 in / 5,253 out / 3 turns vs 沙箱脚本 54,912 in（**-61.6%**）/ 308 out（**-94.1**%）/ 2 turns，答案完全一致 | L3b 沙箱路径成立 |
| CS-2 | 40 事件日志 5 问召回：Grep 58,614 in / 5 次调用全命中（Q5 文件编辑 8/8）；FTS5(BM25) 140,073 in（**贵 2.4 倍**）、4 次调用多次 `(no results)` 重试（Q5 仅 2/8） | 不建索引；events.log 一行一事格式 |
| CS-3 | 跨 hook marker：session 键并行 3 工具 → 1/3 命中 + **错归因**（Read 的 POST 读走 Grep 的 PRE 内容）；tool_use_id 键 → 3/3、125-234ms、零残留 | v1 单向台账绕开整个机制；v1.5 若启用必须 tool_use_id 键 + attribution_ok 校验 + 读后 unlink + unaccounted 计数器 |
| CS-4 | 移植 auto-injection：常规 20 事件 162/500 tok（P1 16/P2 76/P3 44/P4 27）；病态输入（10 条肥胖决策 `'Decision '+i+': '+('very detailed architectural rationale text '.repeat(12))` + 15 个 `'comprehensive-skill-identifier-'+i+'-with-suffix'` 技能 + 意图 `'The user intent statement '.repeat(8)`）5→3 回退后仍 **584 tok，溢出 84**（keptDec 3、keptSkills 10） | §8.3 硬截断 while 循环 |

### 11.3 共性失败模式 → 代码级规避

| 失败模式（出处） | 规避落点 |
|----------------|---------|
| Read 压缩失真（rtk issue #822，3/3 项目踩坑默认关闭） | 只做 skeleton 不做行级正则过滤；逃逸口；<8 行不压缩 |
| BPE 反向变大（实测 48,105→48,222 字节） | tokenizer-count gate：压完变大回退原值 |
| 跨层管道冲突（tee\|grep -c 静默丢警告） | filter 只在 hook 内部，永不进 pipe |
| 召回风暴（rtk ON 时工具调用 +19.5%） | 不自建召回工具，复用原生 Read/Grep |
| 命令改写二次权限检查 | policy/escape.rs 复杂度检查 |
| 注入预算溢出 | inject/budget.rs 硬截断（铁律 8） |
| 台账错归因 | v1 无跨 hook 状态（铁律 7） |
| offset/limit 无限循环 | 逃逸口恒开 |
| 中文丢失 / locale 破坏 | 双语词典 + 禁 LC_ALL |
| 长会话 filler drift（caveman 团队记录，5 轮内未复现） | 每轮强化 + 标记为已知风险 |

---

## 12. 验收场景规格（可独立复建的测试场景，含原始数据）

实现验收时不需要原始 spike 脚本——以下规格足以复建每个场景。

### 12.1 M0 冒烟（复放 S1）

settings.json 允许 `echo ORIGINAL*`；PreToolUse hook 把 `echo ORIGINAL x` 改写为
`echo REWRITTEN x`（updatedInput.command）。预期：无权限弹窗，实际输出 `REWRITTEN x`。
反向用例：改写为 `echo REWRITTEN x | node -e "..."` 必须触发二次权限检查（不许发生——escape.rs 拦截）。

### 12.2 M1 逃逸口（复放 S2b）

200 行文件 bigfile.txt；hook 改写到骨架。然后模型用 `offset=50 limit=10` 重读 →
hook 必须 passthrough（不压缩），ledger 记 retrieval 行。无循环、无重复压缩。

### 12.3 M3 注入（复放 S3 结构）

fixture prompt："explain closures"。四组对照：无注入 / 330 chars / 1500 / 3500+（§8.1 文案）。
参考数据（Claude Code 2.1.117 实测）：output_tokens 505 / 394 / 275 / 179。
验收线：3500+ 组比无注入组 -50% 以上，且回复含 verbatim 代码、无问候语开头。

### 12.4 M3 多轮（复放 S3+ 方法）

`claude -p "<prompt>" --output-format stream-json --verbose` 首轮取 session_id，
后续 `claude -p "<prompt>" --resume <session_id>` 逐轮累积。5 个 prompt：
闭包 / 原型继承 / 事件循环 / async-await / React reconciliation。
A 组仅 SessionStart 注入、B 组加每轮强化。参考数据：A 合计 1225、B 合计 990 out_tok。
注意：`--session-id` 不会跨调用累积（实测失败）；`--input-format stream-json` 会把多条
合并成一个回复（实测失败）——必须用 `--resume`。

### 12.5 M4 锚点（复放 S4 场景）

16 个 200 行文件（data/file_00..15.txt），PreToolUse:Read 骨架化，`--max-turns 30`
让模型逐个读完后总结。预期（实测口径）：模型总结中保留全部 16 个
`[TSK SKELETON of file_NN.txt]` 锚点、不幻觉内容、能说明从锚点可检索原文。

### 12.6 M1 中文回归（复放 S5）

5 条 commit（3 纯中文："修复登录bug" / "新增用户接口" / "重构数据库模块" + 2 英文）
经 TSK 处理的 git 输出：5/5 保留。反例参考：英文 filter 会丢 2/3 纯中文 commit。
全库 grep 断言：不存在 `LC_ALL` 赋值行。

### 12.7 M2 沙箱（复放 CS-1）

fixture：nginx access log 500 行，行格式
`192.168.1.10 - - [15/Sep/2026:10:00:01 +0000] "GET /api/users HTTP/1.1" 200 5120 "ref" "UA" 0.045`
（含若干 404/500/慢请求）。沙箱脚本只 print 6 行统计：Total requests / 各状态码计数 /
Top 3 paths / Total bytes / 平均响应时间 / 慢请求数。验收：答案与直接 Read 组一致，
输入 token 相对直接 Read -50% 以上（参考：-61.6%）。

### 12.8 M1 召回格式（复放 CS-2 fixture）

40 事件写入 events.log（一行一事）。canonical fixture（决策 8 条，实测用）：

```
decision: use Rust core engine, not Go, for performance
decision: fail-open on all hook errors, never block session
decision: skeleton compression keeps head 5 + tail 3 lines
decision: auto-compaction is unreliable at 200k boundary, use threshold advice
decision: LC_ALL=C must NOT be forced, breaks Chinese locale
decision: SQLite FTS5 replaced by session directory files + path hints
decision: PreCompact hook preserves TKS skeleton markers verbatim
decision: output compression defaults to OFF, user opts in
```

（errors/file-edits/prompts/tool-results 8 条各一组，同构造即可。）
5 问召回（Grep 一次命中为验收）：Rust 决策原因 / tee|grep 错误文本 / 输出压缩默认态 /
tiktoken 错误 / "modified compression level logic" 编辑过哪些文件（8 个文件全中）。

### 12.9 M4 预算（复放 CS-4 病态输入）

`assemble()` 喂病态数据（逐字复刻，与 §11.2 同源）：

```
decisions = ['Decision '+i+': '+('very detailed architectural rationale text '.repeat(12)) for i in 0..10]
skills    = ['comprehensive-skill-identifier-'+i+'-with-suffix' for i in 0..15]
role      = 'Rust CLI token-reduction plugin for Claude Code agent sessions'
intent    = 'The user intent statement '.repeat(8)
```

断言：
- 原版算法（无硬截断）产出 584 tok / fallback=true / keptDec=3 / keptSkills=10 ——这是回归测试的靶子；
- TSK 算法（§8.3）产出 est ≤500、溢出=0。
常规 20 事件参考：162 tok，P1 16 / P2 76（5/10）/ P3 44（8/8）/ P4 27。

### 12.10 marker 机制（仅 v1.5 参考，v1 不实现）

若未来启用跨 hook marker：PreToolUse 写 `tmpdir/tsk-marker-<tool_use_id>.txt`，
PostToolUse 读取→校验内容 tool 名与事件一致（attribution_ok）→unlink→入账；
读不到记 unaccounted。参考数据：session 键并行 3 调用 1/3 命中 + 错归因；
tool_use_id 键 3/3、125-234ms。

---

## 13. 测试策略（CI 必跑 7 组）

| 组 | 内容 | 守护 |
|----|------|------|
| 确定性黄金 | 全部 fixture → 与 tests/golden/ 逐字节相等 | 铁律 2 |
| fail-open 黄金 | 损坏 JSON / panic / 不存在文件 / 超时 → stdout 恒 `{}`、exit 恒 0 | 失败模型 |
| 决策表 | policy 输入特征 × 期望 Plan | S2b/S1 |
| 安全 | 黑名单变量探测脚本拿不到值；代理变量清除；超时 kill 进程树 | §9 |
| 预算 | §12.9 病态回归：注入 ≤500，溢出=0 | 铁律 8 |
| 中文 | §12.6 + 全库无 LC_ALL 赋值 | S5 |
| 端到端 | assert_cmd 喂 stdin 断言 stdout，5 类事件各≥1 例 + §12 场景 | 协议 |

---

## 14. 里程碑与前置穿刺

| 里程碑 | 周期 | 交付 | 关键验收 |
|--------|------|------|---------|
| M0 协议骨架 | 1w | 5 类事件分发 + fail-open + doctor + §12.1 冒烟 | fail-open 黄金测试进 CI |
| M1 输入压缩 | 2w | skeleton + 逃逸口 + ledger + report 最小版 | §12.2 / §12.6 / §12.8 / 确定性逐字节 |
| M2 沙箱+外置 | 2w | tsk exec + 路由 + >100KB 外置 | §12.7 + 安全组 + 回退链 |
| M3 注入面 | 1.5w | ruleset 资产 + 每轮强化 + auto-clarity + 180k 建议 | §12.3 / §12.4 / 默认关 |
| M4 快照+预算 | 1.5w | snapshot + budget + cache-hit 指标 | §12.5 / §12.9 / 手动 /compact 端到端 |
| M5 发布 | 1w | 真实负载验证 + 合规定稿 | ledger vs API usage 10 会话；发布清单 §16 |

**前置穿刺（进对应里程碑前跑）**：
- **P-1（阻塞 M2，最高优先）**：沙箱任务判定——20 个真实任务样本标注"精读 vs 统计"意图，测判定准确率。判定规则落地前路由保守默认（§7.1）。
- P-2（M3 前）：auto-clarity 触发——mock DROP TABLE / rm -rf 场景，验证注入降级为清晰文风。
- P-3（M2 前）：10MB 文件 skeleton + 沙箱冷启动基准——hook 有 5s 预算。

---

## 15. 遗留问题（实现期随时核对）

| # | 问题 | 阻塞 |
|---|------|------|
| 1 | 20+ 轮长会话漂移未测 | 不阻塞（输出压缩默认关） |
| 2 | auto-clarity 触发正确性 | M3（P-2） |
| 3 | /compact 后 PreCompact 实际触发 + 快照→注入端到端（S4 中 PreCompact 从未真实触发过，§5.1 的 PreCompact stdin 字段是文档口径需实测） | M4 验收 |
| 4 | 多 agent hook 协议差异 | v2 |
| 5 | 10MB 性能（hook 5s 预算） | M2（P-3） |
| 6 | wenyan 档位（文言文压缩，caveman 已验证但中文场景适配未测） | 发布后再评估（v1 不做） |
| 7 | cache 命中率 CI 指标实现（铁律 6 要求，M4 落地 `tsk report --cache-hit`） | M4 |
| 8 | 沙箱任务判定规则 | **M2 前（P-1）** |
| 9 | 台账 ~5% 误差真实负载验证 | M5 |
| 10 | v1.5 marker 启用判定（前提三选一：CI 精确对账 / 实测误差显著偏大 / 需要 PostToolUse 数据） | 发布后 |
| 11 | 沙箱进程级网络隔离（当前仅清代理变量，防注入不防外连） | v2 |
| 12 | LICENSE 官方全文未落盘 | **发布前必须（§16）** |
| 13 | Bash 大输出外置（PostToolUse 不可改写 tool_result，v1 无解） | 平台限制，v2 评估 |

---

## 16. 开源合规

### 16.1 License

**Apache-2.0**。`LICENSE` 必须是官方全文：发布前从
`https://www.apache.org/licenses/LICENSE-2.0.txt` 下载落盘
（法律文本禁止凭记忆抄写/由模型生成）。核对首行 `Apache License`、版本行 `Version 2.0, January 2004`。

### 16.2 NOTICE（全文，发布时逐字使用）

```text
TSK (Token Saving Kit)
Copyright 2026 TSK contributors

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

This product includes software developed as part of the TSK project.
TSK is a new implementation written from scratch. It does not
contain, link, or redistribute source code from any project
acknowledged below. Entries marked [Adapted logic] follow the
behavioral approach of the referenced project and were independently
re-expressed in Rust by TSK authors; per good open-source practice
and the terms of the applicable licenses, we acknowledge those
origins here.

=====================================================================
ACKNOWLEDGMENTS
=====================================================================

TSK's design was informed by studying the following open-source /
source-available projects. We thank their authors. Each entry states
what TSK drew from the project. License information was verified
against each project's LICENSE file at study time (2026-09).

---------------------------------------------------------------------
rtk (Rust Token Killer) — Apache-2.0
---------------------------------------------------------------------
Drew from rtk (ideas and lessons learned, no code):
- The PreToolUse input-rewrite approach for tool-result compression.
- The "recall storm" lesson: injecting custom recall tools increased
  tool calls (+19.5% in rtk's own measurements), which motivated
  TSK's decision to reuse the agent's native Read/Grep instead of
  building a retrieval index.
- The locale lesson: forcing LC_ALL=C breaks non-English (Chinese)
  git output; TSK preserves system locale and uses bilingual
  keyword dictionaries.

---------------------------------------------------------------------
caveman — MIT (hooks and skills portions)
https://github.com/JuliusBrussee/caveman
---------------------------------------------------------------------
Note: caveman uses a split license. The portions TSK studied —
src/hooks/ (caveman-activate.js, caveman-mode-tracker.js) and
skills/ (SKILL.md) — are within the MIT-licensed adoption surface.
The BSL-1.1-licensed engine directories were not studied or used.
Drew from caveman (ideas, no code):
- The three-layer injection architecture for output style
  compression: one long SessionStart injection (~3,500 chars), short
  per-turn reinforcement, and re-emission on compaction/resume.
- The finding that short "be terse" injections are ignored by
  models, and that rules with positive/negative examples and a
  self-check directive are required (verified independently by
  TSK's own spike experiments S3/S3+).
- The auto-clarity concept: automatically dropping compressed style
  for safety-critical or irreversible-action content.
TSK's injection texts are original works authored for TSK.

---------------------------------------------------------------------
context-mode — Elastic License 2.0 (ELv2), source-available
https://github.com/mksglu/context-mode
---------------------------------------------------------------------
context-mode is licensed under ELv2, NOT an OSI open-source license.
TSK does not contain or redistribute any context-mode code; ELv2's
restrictions (no hosted service offering, no license-notice removal)
apply to context-mode's own code and do not restrict TSK, which is
an independent implementation. TSK drew from studying its published
source (ideas and adapted logic, no code copied):
- The sandbox analysis paradigm ("think in code"): run analysis in a
  sandboxed executor; only stdout conclusions enter the model's
  context. TSK's sandbox is an independent Rust implementation.
- The environment-variable denylist approach for sandbox hardening.
  TSK compiled its own denylist from the same public injection-vector
  classes (BASH_ENV, NODE_OPTIONS, LD_PRELOAD, PYTHONSTARTUP, ...).
  [Adapted logic]
- The 100KB large-output externalization threshold: tool results
  above this size are replaced by a pointer + summary. [Adapted
  logic]
- The PreCompact resume-snapshot (<2KB) and post-compaction
  auto-injection with a priority budget (role / decisions / skills /
  intent, ~500 token cap). TSK adds a hard-truncation pass that the
  original lacks (TSK experiment CS-4 found the original cap is
  soft and overflows on adversarial input). [Adapted logic]
- The "stdout write is the LAST action" hook discipline.
- Negative findings verified by TSK's own experiments on a
  re-created replica of the mechanism (CS-3): session-keyed
  cross-hook marker accounting suffers parallel-collision and
  misattribution bugs under concurrent tool calls. TSK's v1 avoids
  the mechanism entirely (single-writer append-only ledger).

---------------------------------------------------------------------
context-compress — MIT
---------------------------------------------------------------------
Drew from context-compress (lessons learned, no code):
- The BPE reverse-enlargement failure: BPE token compression can
  increase byte count on some unicode texts (48,105 -> 48,222 bytes
  observed in its benchmarks). TSK implements a tokenizer-count
  gate: if compression grows the content, keep the original.

---------------------------------------------------------------------
headroom — Apache-2.0
---------------------------------------------------------------------
Drew from headroom (ideas, no code):
- Context-window monitoring as a first-class feature.

---------------------------------------------------------------------
ctxpress — MIT (declared in README; no LICENSE file in repo at
study time)
---------------------------------------------------------------------
Drew from ctxpress (ideas, no code):
- Layered input compression and tiered cache thinking, used as
  cross-comparison reference.

---------------------------------------------------------------------
toon — MIT
---------------------------------------------------------------------
Studied for compaction hook design; no specific logic adapted.

---------------------------------------------------------------------
glyphdown — PolyForm Noncommercial 1.0.0
---------------------------------------------------------------------
glyphdown is under a non-commercial license. TSK studied it for
cross-comparison ONLY: it was evaluated and explicitly NOT adopted
(requires fine-tuned models, incompatible with TSK's requirements).
TSK contains no glyphdown logic or derived work. No TSK feature
derives from it. Listed here for transparency about the survey.

---------------------------------------------------------------------
steno — no license declared (all rights reserved by default)
---------------------------------------------------------------------
steno's repository declares no license. Accordingly TSK borrowed
nothing from it — no code, no logic, no design. It was surveyed for
cross-comparison only, at the level of its public README
description. Listed here for transparency about the survey.

=====================================================================
EXPERIMENT DATA
=====================================================================
All spike experiments referenced in TSK documentation (S1-S5, S3+,
CS-1..CS-4) were designed and executed by the TSK project on
Claude Code 2.1.117. Brief quoted remarks from the projects above
(e.g., caveman's code comments on injection weakness) are short
attributions used with source cited in the accompanying design
documentation; TSK's re-created test replicas (e.g., the CS-3
marker mechanism, the CS-4 budget assembler) are original TSK work.
```

### 16.3 仓库红线与发布清单

- 任何第三方项目的源码树（caveman-main/、context-mode-main/ 等）**不得进入** TSK 仓库；引用过其片段的实验日志发布前清理。
- 发布清单：
  1. LICENSE 官方全文落盘（§16.1）
  2. NOTICE 复核——对照各参考仓库**当时最新**的 LICENSE（协议会变：调研期就发现 3 处记录与实际不符）
  3. crates.io / GitHub 重名检索（"TSK" / "token-saving-kit"）
  4. 敏感信息扫描（无凭证、无内部路径）
  5. README 指向本文档

---

## 17. 实现顺序（自底向上，每步可测）

```
 1. events.rs + cli 骨架        → M0：喂假 JSON 断言 {} + exit 0（§13 fail-open 组先行）
 2. storage.rs + config.rs      → 目录/配置就位，doctor 可测
 3. compress/skeleton.rs        → 纯函数单元测试先行（铁律 2 从第一天就测；格式 §6.1）
 4. policy/escape.rs            → 决策表测试（§12.2）
 5. PreToolUse(Read) 串联       → §12.1/12.2 场景（真实 claude -p 冒烟）
 6. ledger.rs                   → 追加写测试（含并发打开两次）；report 最小版 → M1 验收
 7. sandbox/env.rs + limits.rs  → 安全测试组（§9 探测脚本）；P-3 性能穿刺
 8. policy/routing.rs           → P-1 穿刺先行；路由决策表
 9. compress/externalize.rs     → §12.7 场景复现 → M2 验收
10. inject/ 静态资产             → §8.1 文案落文件 + 长度断言；§12.3/12.4 → M3 验收
11. inject/budget.rs            → §8.3 算法 + §12.9 病态回归测试先行（TDD）→ M4
12. snapshot.rs + PreCompact    → 手动 /compact 端到端（遗留 #3 实测核对 §5.1 字段）
13. cache-hit + report 完整版   → 铁律 6
14. M5：真实负载 10 会话 + 合规发布清单（§16.3）
```

每步都有对应的验收场景编号（§12）与测试组（§13）——不存在"写完再补测试"的步骤。

---

*TSK v1 实现手册 · 2026-09 · 依据 9 项穿刺（S1-S5/S3+/CS-1..4）· 平台口径 Claude Code 2.1.117*