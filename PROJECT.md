# TSK 工程文档

> TSK = **T**oken **S**aving **K**it（Token 节省工具包）。面向 Claude Code（Anthropic 的命令行 AI 编码代理）的 token 节省插件，Rust 核心 + 代理钩子（Hooks）。
>
> 本工程在 58 个自动化测试（17 单元 + 10 集成 + 31 冒烟）全绿、真实会话验证通过的状态下，处于可发布阶段。

---

## 1. 介绍

### 1.1 它解决什么问题

LLM（Large Language Model，大语言模型）会话的成本与「上下文窗口」（Context Window，模型单次能处理的最大 token 数）直接相关。token（Token，模型处理文本的最小单元，约 3-4 字符/token）消耗主要来自三处：

- **输入侧**：agent（Agent，能自主调用工具完成任务的 AI 程序）读取文件、执行命令的结果全部进入上下文；
- **输出侧**：模型生成的回答文风越啰嗦，输出的 token 越多；
- **历史侧**：长会话中上下文被旧内容占满，触发压缩（Compact）时信息可能丢失。

TSK 对这三条线各做一次压缩，目标是**在用户无感知的前提下降低 token 消耗**——用户不需要敲任何"压缩 token"的命令，压缩由钩子自动完成。

### 1.2 三条压缩线

| 压缩线 | 机制 | 默认状态 | 是否无损 |
|---|---|---|---|
| 输入 | 文件骨架化（Skeleton）/ 大文件外置（Externalize）/ 沙箱分析（Sandbox） | 开 | 无损（原文可经逃逸口/外置文件回读） |
| 输出 | 注入文风规则（Ruleset）改变回答风格 | **关**（需显式开启） | 无损（改变文风，不改正确性） |
| 历史 | 压缩前快照（Snapshot）+ 恢复注入（Resume） | 开 | 无损（信息提炼） |

### 1.3 关键名词速查

| 缩写 | 全拼 | 中文 |
|---|---|---|
| TSK | Token Saving Kit | Token 节省工具包 |
| LLM | Large Language Model | 大语言模型 |
| agent | — | 智能体（自主调用工具的 AI 程序） |
| token | — | 模型处理文本的最小单元（估算法：`bytes/4` 向上取整，记 est tokens） |
| hook | — | 钩子（平台在特定时机回调外部命令的机制） |
| PreToolUse | — | 工具调用**前**（钩子事件） |
| PostToolUse | — | 工具调用**后**（钩子事件） |
| SessionStart | — | 会话开始（钩子事件） |
| UserPromptSubmit | — | 用户消息提交（钩子事件） |
| PreCompact | — | 上下文压缩前（钩子事件） |
| context window | — | 上下文窗口（模型单次可处理的 token 上限） |
| compact | — | 上下文压缩（提炼旧内容、释放空间） |
| skeleton | — | 骨架（保留文件首尾、省略中部的压缩产物） |
| externalize | — | 外置（大文件全文落盘、上下文只进摘要与指针） |
| escape hatch | — | 逃逸口（`offset`/`limit` 参数恒放行的无损取回机制） |
| sandbox | — | 沙箱（隔离环境执行分析脚本） |
| ledger | — | 台账（单向追加的记账文件） |
| fail-open | — | 失败放行（出错时保持原样、绝不阻塞会话） |
| break-even | — | 盈亏平衡点（收益刚好覆盖成本的临界轮数） |
| cache hit | — | 缓存命中（`cache_read_input_tokens` 占比） |
| resume | — | 恢复会话（`SessionStart(source=resume)`） |
| CI | Continuous Integration | 持续集成 |
| CLI | Command Line Interface | 命令行接口 |

---

## 2. 功能性

### 2.1 输入压缩（无损）

| 功能 | 触发条件 | 行为 | 实测收益 |
|---|---|---|---|
| **Skeleton 骨架** | Read 的文件 ≥8 行、≤100KB，且骨架比原文小（反膨胀门控） | 上下文只进文件**首 5 行 + 尾 3 行** + 省略标记（标记写明如何取回中间行） | 原文 **-64% ~ -97%**（99 行 -64% / 200 行 -83% / 999 行 -97%） |
| **Externalize 外置** | Read 的文件 >100KB | 全文落盘到 `ext/<sha1>.txt`，上下文只进 **1KB 摘要 + 指针**；模型需全文时按指针回读 | **-99.4%**（201786B → 1275B） |
| **Sandbox 沙箱分析** | `tsk exec`（显式）或 P-1 自动路由（**50–100KB** 且扩展名 ∈ {log,jsonl,csv,json,tsv} 且项目预置 `analyze_<ext>` 脚本；>100KB 恒外置优先） | 脚本在隔离环境运行，只把 stdout（结论）进上下文；>64KB 结论截断外置 | **≈ -100%**（500 行日志 → 6 字节结论） |
| **Escape hatch 逃逸口** | Read 带 `offset` 或 `limit` 参数 | 恒放行不压缩，模型可逐行取回原文（无损性保证） | 取回成本 ~90B/次 |

**无损性**：所有输入压缩都保证原文件仍在、中间行可经逃逸口取回、外置全文可逐字节回读、中文内容逐字保留。

### 2.2 输出压缩（无损但用户感知，默认关）

| 功能 | 行为 | 实测收益 |
|---|---|---|
| **Ruleset 规则集**（SessionStart 注入） | 注入 3867 字符的文风规则（硬规则 + 正反例 + 自检 + 安全例外 AUTO-CLARITY），要求回复去客套、碎片化、代码逐字保留 | 真实 A/B：输出 **814 → 254 tok（-68.8%）** |
| **Reinforce 每轮强化** | 每轮用户消息注入 ~200 字符强化指令 | 长注入之上再 **-19%**（spike 数据） |
| **Resume 恢复注入** | 会话恢复时注入提炼的快照（≤500 tok 预算） | est 49 tok/次 |

| 档位 | 行为 | 注入成本 | 压缩效果 | 用户感知 |
|---|---|---|---|---|
| `off` | 不压缩输出（默认） | 0 | 无 | 否 |
| `lite` | 短规则集（1443 字符） | est 361 tok/会话 | 较弱（S3: ~1500 字符 -46%） | 是（文风简） |
| `full` | 完整规则集（3867 字符） | est 967 tok/会话 | 最强（真实 A/B **-68.8%**） | 是（文风简） |
| `ultra` | 同 full（预留更激进档） | est 967 tok/会话 | 同 full | 是（更简） |
| `auto` | 按会话轮数自动启停 | 短会话 0；长会话同 full | 长会话同 full | 开了才生效 |

> **口径**：`output_compression` 是唯一改变回答文风（用户可感知）的选项，但技术正确性始终无损（代码/命令/路径逐字保留、AUTO-CLARITY 安全例外）；输入压缩与历史压缩两条线同样无损。

**auto 档（break-even 自动启停）**：输出压缩有固定的输入侧成本（规则集 est 967 tok + 每轮强化 est 88 tok）。实测 break-even ≈ 3 轮：**1 轮净亏 -495 tok、2 轮净亏 -23 tok、3 轮起净赚**（3 轮 +449、10 轮 +3753 tok）。auto 档在前 2 轮不注入（短会话零成本），第 3 轮起自动开启。

### 2.3 历史 / 上下文管理（无损）

| 功能 | 行为 |
|---|---|
| **PreCompact 快照** | 上下文压缩前提取会话的 role（角色）/ decisions（决策）/ skills（技能）/ intent（意图）写入 <2KB 快照 + 锚点保护指令（保留骨架标记行） |
| **Compact 建议** | 上一轮输入 >180k token 时，注入一行 `/compact` 建议（幂等，只建议一次） |
| **Resume 恢复** | 会话恢复时把快照组装成恢复注入，减少重新探索 |

### 2.4 沙箱（`tsk exec`）

隔离环境执行分析脚本，只把结论送进上下文。安全基线：环境变量黑名单（清除 BASH_ENV、NODE_OPTIONS、LD_PRELOAD 等注入向量）、网络代理清除、输入 >100MB 拒绝、输出 >64KB 截断外置、10 秒超时杀进程树、stderr 隔离。

### 2.5 运维

| 命令 | 功能 |
|---|---|
| `tsk init` | 安装：向 `<project>/.claude/settings.json` 合并 5 个钩子段 + 写 `~/.tsk/config.yaml`（幂等，可重复） |
| `tsk off` | 一键关闭：`enabled` 置 false + 摘除钩子（幂等，可重开） |
| `tsk report` | 台账聚合（默认当前会话）：节省字节+est tokens、注入成本、净值、按工具分布；`--all` 全量、`--clean` 清空、`--cache-hit` 命中率 |
| `tsk doctor` | 自检：配置合法性、目录可写、钩子接线、PATH（可执行搜索路径） |
| `tsk exec` | 沙箱执行脚本 |
| `tsk hook <event>` | 钩子入口（agent 调用，不面向用户） |

### 2.5 收益口径（诚实说明）

所有量化数字（`report` 的 saved、本文档各收益表）都是**纯机制节省**：压缩产物 vs 全文的字节差。
**不含"模型若需要全文时经逃逸口取回的额外开销"**——模型是否需要全文是运行时才知道的，`report` 报的是客观的压缩产物对比。模型确需全文时取回是额外成本，已在测试如实量化（`escape_retrieval_cost`）：

#### 假设读取一个长度为423token的文本

| 场景 | 成本 |
|---|---|
| 只要摘要：读骨架 | est 73 tok |
| 要中间 2 行：骨架 + 2 次取回 | est 118 tok |
| 要全文：骨架 + 5 次取回 | est 635 tok（直读全文只要 423）→ **额外 +212 tok（+50%）** |

结论：压缩省 token 的前提是模型不需要读全文；`report` 数字应理解为"模型按提示高效使用骨架/摘要"情景下的节省，不隐藏取回代价。  `tsk report` 末尾会直接给出**悲观上限**：被压缩原文合计 + 「若全部回读原文全文」的额外输入与净省（回读时净省趋近或为负），把该回读成本如实摆在报告里。

> 下表每行收益都对**某特定输入规模**而言（例如"200 行文件"骨架化、"4000 行/201KB 日志"外置、单轮 prompt "explain closures" 的输出 A/B、30 轮长会话累计）。收益幅度随输入规模、文件类型、模型与任务而变，同规模才可比；token 口径统一 est_tokens = ⌈bytes/4⌉。

### 2.6 实测收益总表

| 压缩线 | 场景 | 原来 | 压缩后 | 收益 |
|---|---|---|---|---|
| 骨架 | 200 行文件 | est 423 tok | est 71 tok | **-83.3%** |
| 外置 | 4000 行日志 201786B | est 50447 tok | est 319 tok | **-99.4%** |
| 沙箱 | 500 行 nginx 日志 | est 11946 tok | est 6 tok | **≈ -100%** |
| 输出 | 真实 A/B（同一 prompt） | 814 tok | 254 tok | **-68.8%** |
| 30 轮长会话 | 混合任务 | est 408720 tok | est 3126 tok | **-99.2%**（净省 403k） |
| 真实工程 30 轮 | 复制工程定位/修 bug | est 41731 tok | est 3072 tok | **-92.9%**（绝对值 155034B） |
| 真实 claude 30 轮 | 同工程同任务 | 输出 35621 / 输入 908k | 输出 8732 / 输入 309k | **输出 -75.5% / 输入 -66%** |

---

## 3. 架构

### 3.1 五层架构

```
L1 Agent Hooks      events.rs（钩子协议）+ CLI（stdio 编排）
L2 Policy           policy/：路由、逃逸口判定、沙箱候选判定
L3a Skeleton        compress/skeleton.rs（≤100KB 精读路径）
L3b Sandbox         sandbox/（隔离执行、env 过滤、超时）
L3c 注入/外置/快照  inject/ + compress/externalize.rs + snapshot.rs
L4 Storage+记账     storage.rs + ledger.rs
```

### 3.2 模块结构

```
tsk/
├── Cargo.toml            # workspace（core + cli）
├── core/                 # tsk-core：纯库，全部业务逻辑
│   └── src/
│       ├── config.rs     # 用户层 2 键配置 + 工程常量
│       ├── events.rs     # 钩子事件协议类型
│       ├── policy.rs     # 决策表（骨架/外置/沙箱路由、逃逸口）
│       ├── compress/     # skeleton.rs + externalize.rs
│       ├── sandbox.rs    # 沙箱执行 + env 黑名单 + 超时
│       ├── inject.rs     # 三线汇聚：输入改写、输出注入、快照建议
│       ├── snapshot.rs   # 快照提取 + 恢复注入组装（预算硬截断）
│       ├── ledger.rs     # 单向台账
│       └── storage.rs    # 目录布局
├── cli/                  # tsk-cli：命令行入口（clap + stdio 编排）
├── examples/sandbox/     # 沙箱分析脚本模板
└── tools/smoke_table.py  # 冒烟结果表格工具
```

**分工纪律**：`cli` 不含业务逻辑，只编排 stdio；`core` 是纯库；错误用 `Result` 一路传播，`cli::main` 是唯一 catch 点。

### 3.3 存储布局

```
~/.tsk/                     # 用户级（机器私有）
├── config.yaml             # 两键配置
└── <session_id>/           # 会话数据
    ├── ledger.jsonl        # 单向台账（每动作一行）
    ├── resume-snapshot.json# 压缩前快照（<2KB）
    ├── events.log          # Grep 友好事件日志（一行一事）
    └── ext/<sha1>.txt      # 外置大输出

<project>/.tsk/skeletons/   # 项目级工件（可 gitignore）
└── <sha1>-<basename>.skeleton.txt   # 骨架缓存（同路径同骨架，确定性）
```

### 3.4 钩子数据流

5 类钩子事件：`PreToolUse`（工具调用前，唯一改写点）、`PostToolUse`（工具调用后，恒放行 + 观测）、`SessionStart`（注入规则集/恢复）、`UserPromptSubmit`（每轮强化 + compact 建议）、`PreCompact`（快照 + 锚点保护）。

stdout 协议（每次钩子必须输出完整合法 JSON）：改写返回 `hookSpecificOutput.updatedInput`，注入返回 `hookSpecificOutput.additionalContext`，无动作返回 `{}`。**stdout 永远最后写、永远是完整 JSON**——半截 JSON 比不输出更糟。

### 3.5 失败模型

**Fail-open（失败放行）**：任何钩子出错 → 向 stderr 记一行 `[tsk] fail-open: …`，stdout 返回 `{}`、exit 0——本轮无 TSK，会话照常。宁可少省 token，绝不阻塞。实测矩阵：坏 JSON / 未知事件 / 空输入 → 全部 `{}` + exit 0。

---

## 4. 技术细节

### 4.1 平台机制约束（实测确认，不可违反）

| 约束 | 实测来源 | 实现含义 |
|---|---|---|
| 权限按原命令判定，改写发生在判定之后 | S1 | 改写不变复杂度 ⇒ 免权限弹窗 |
| 改写后命令复杂度超过原命令会触发二次权限检查 | S1 后续 | **Bash 工具事件恒放行**（任何有收益的改写都需管道/多语句 ⇒ 必弹窗） |
| `PostToolUse` 无法改写 `tool_result`（工具结果） | S2 | 一切改写只能在 `PreToolUse`；`Bash`/`Grep` 的大输出无法事后压缩 |
| 带 `offset`/`limit` 的重读会与改写形成无限循环 | S2b | 逃逸口恒开、不可配置 |

因此 v1 压缩面为：`Read` 的路径改写（骨架/外置）+ 输出文风注入 + 沙箱分析。**`Bash` 输出压缩不在 v1 处理**——平台锁死，这是已知限制而非遗漏。

### 4.2 Skeleton 文件格式（确定性生成）

```
[TSK SKELETON of <相对路径> | <N> lines total | head 5 + tail 3 | TSK v1]
     1	<第 1 行原文>          ← 6 列右对齐行号 + TAB + 原文
     2	<第 2 行原文>
     5	<第 5 行原文>
[TSK omitted lines 6..<N-3>. Full file: Read <路径> with offset/limit, or Grep <pattern> <路径>]
 <N-2>	<倒数第 3 行原文>
 <N>	<最后一行原文>
```

确定性保证：同输入永远同字节（CI 逐字节比对）。反膨胀门控：骨架不小于原文时回退原文件（防止小文件被骨架 overhead 反超）。

### 4.3 台账格式（ledger.jsonl，单向追加）

每行一个事件，kind（类型）区分：`rewrite`（骨架）、`retrieval`（逃逸口取回）、`externalize`（外置）、`inject`（注入）、`sandbox`（沙箱）。示例：

```json
{"kind":"rewrite","ts":1789473338663,"session":"...","tool":"Read","tu_id":"...","strategy":"skeleton","orig_bytes":143200,"new_bytes":647,"saved":142553}
```

记账纪律：纯追加、单写者、无读取回路；写失败只记日志不阻断。

### 4.4 沙箱安全基线

- **env 黑名单**：清除所有可导致解释器启动执行任意代码/加载额外库/劫持模块路径的变量（BASH_ENV、NODE_OPTIONS、PYTHONSTARTUP、LD_PRELOAD、GIT_CONFIG_KEY_* 等按前缀匹配）；白名单转发 PATH、HOME、LANG 等。**全代码库禁止设置 `LC_ALL=C`**（会破坏中文输出）。
- **网络 deny**：清除代理变量、不给凭证（进程级网络隔离：Linux `TSK_NET_ISOLATE=1` 用 `unshare -n`，Windows 无此机制）。
- **硬顶**：输入 >100MB 拒绝；输出 >64KB 截断 + 全文外置；10s 超时杀进程树；stderr 不进上下文。

### 4.5 输出压缩的 break-even（成本/收益模型）

| 轮数 | 注入成本（实测） | 输出节省（实测 A/B） | 整体 |
|---|---|---|---|
| 1 | 1055 tok | 560 tok | **-495 净亏** |
| 2 | 1143 | 1120 | **-23 净亏** |
| 3 | 1231 | 1680 | **+449 净赚** |
| 10 | 1847 | 5600 | +3753 |

结论：≤2 轮短会话应关闭输出压缩（auto 档自动处理）；≥3 轮净赚。

### 4.6 已知平台事实

- **auto-compact 不触发 PreCompact 钩子**（真实会话 8 次反复实测确认）：Claude Code 执行上下文压缩（`compact_boundary` 事件，pre_tokens 119199→100963）但不调用 PreCompact 钩子。因此 TSK 的 PreCompact 快照只在**手动交互式 `/compact`**（需 TTY 交互终端）时触发；快照机制的钩子侧全链路已由冒烟测试覆盖。
- **输出压缩注入反提高缓存命中**：真实 30 轮，TSK on 的 cache read 占比 95.6%（off 为 76.2%）——规则集文本固定，多轮命中缓存。

---

## 5. 如何测试

### 5.1 运行全部测试

```bash
cargo test                       # 17 单元 + 10 集成 + 31 冒烟 = 58，全绿
cargo test --test smoke -- --nocapture   # 冒烟带量化输出
python tools/smoke_table.py      # 冒烟执行结果表格（测试名|测试点|结果|收益/成本）
```

### 5.2 测试分层

| 层 | 数量 | 覆盖 |
|---|---|---|
| 单元 | 17 | 决策表、骨架格式、外置、预算硬截断、沙箱 env 过滤、中文、无 LC_ALL 强制 |
| 集成（e2e） | 9 | 真实 stdin/stdout 协议、init/off/report/doctor、cache-hit |
| 冒烟 | 31 | 全部功能面 + 量化收益：骨架多长度、外置无损、沙箱 env/解释器/统计/超时/stderr 隔离、输出压缩三档+auto 档+break-even、逃逸口矩阵+取回成本、30 轮会话、真实工程 30 轮、/compact 闭环、并发 ledger、CRLF/空行边界 |

### 5.3 关键测试设计

- **测能力不测覆盖**：每个测试一个功能点 + 真实字节断言，不做行覆盖追求。
- **确定性**：同输入逐字节相同（骨架黄金格式）。
- **fail-open 矩阵**：坏输入恒 `{}` + exit 0。
- **量化断言**：每个功能带收益阈值（如骨架 ≥50%、外置 ≥98%、沙箱 ≥95%）。
- **无损性**：骨架随机行取回 == 原文、外置全文逐字节 == 原始、中文逐字保留。
- **可靠性**：并发 8 进程写台账 0 坏行、init/off 幂等。

### 5.4 真实会话验证（已执行）

- 骨架改写真实生效：模型 Read 文件收到 `[TSK SKELETON of …]` 而非原文。
- 逃逸口真实取回：模型 `offset:50,limit:1` 取回原文件第 50 行。
- 外置真实回读：模型按指针 `full:` 路径 Read 回全文。
- 输出压缩 A/B：同一 prompt，输出 814→254 tok（-68.8%）。
- 30 轮长会话：输出 -75.5%、输入 -66%、无文风漂移、cache 命中反升。

---

## 6. 如何安装使用

### 6.1 从源码安装（非插件）

源代码树安装 = 非插件方式（在项目里 `tsk init` 接线）。安装脚本**总是重新编译最新版本**，不会使用任何可能过期的预编译产物。

```bash
# macOS/Linux : ./install.sh        （Windows: install.bat）
# 脚本：重编 latest → tsk 到 ~/bin → 自动把 ~/bin 加进 PATH（无需手动改 PATH）
# 装好后在项目里接线（零输入）：
cd your-project && tsk init
```

`tsk init` 两件事：① 向 `<project>/.claude/settings.json` **合并** 5 个钩子段（保留已有关键，损坏文件报错不覆盖）；② 写 `~/.tsk/config.yaml`。

> **终端用户装 release 插件包**（hooks + 斜杠命令，最省事）：见 README「二、安装」。另有预编译 release 二进制包 `dist/tsk-v0.1.0.zip`（其包内 install 用绑定二进制、不重编）。
> 从源码直接构建（供贡献者）：`cargo build --release`（产物 `target/release/tsk`）。依赖纪律：仅 `serde / serde_json / serde_yaml / sha1 / clap`（测试用 `assert_cmd`）；不引入异步运行时、不引入 LLM SDK——token 估算用 `bytes/4`，不需要分词器。

### 6.2 唯一配置决策

编辑 `~/.tsk/config.yaml`：`output_compression` 是唯一改变回答文风（用户可感知）的选项，默认 `off`。档位：

| 档位 | 行为 | 注入成本 | 效果 | 用户感知 |
|---|---|---|---|---|
| `off` | 不压缩输出（默认） | 0 | 无 | 否 |
| `lite` | 短规则集 | est 361 tok/会话 | 较弱 | 是 |
| `full` | 完整规则集 | est 967 tok/会话 | 最强（-68.8%） | 是 |
| `ultra` | 同 full | est 967 tok/会话 | 同 full | 是 |
| `auto` | 按会话轮数启停（≤2 轮关、≥3 轮开） | 短会话 0 | 长会话同 full | 开了才生效 |

本项保证技术正确性无损（代码/命令/路径逐字、AUTO-CLARITY 安全例外）；输入/历史两条线同样无损。

### 6.3 一键关闭 / 重开

```bash
tsk off       # 幂等：enabled=false + 摘除钩子，零残留
tsk init      # 重开：重新接线（幂等，不重复）
```

### 6.4 日常使用

无任何"压缩 token"的命令，压缩全由钩子完成。可选检查：

```bash
tsk report                 # 当前会话节省（saved + 注入成本 + 净值 + est tokens）
tsk report --all           # 全量
tsk report --clean         # 清空统计（删除所有台账 + 当前会话标记，不可逆）
tsk doctor                 # 自检
tsk exec examples/sandbox/analyze_log.py access.log   # 手动沙箱分析
```

### 6.5 作为 Claude 插件（可选）

- **终端用户**：装 release 插件包 `dist/tsk-plugin-v0.1.0.zip`（包内自带 install，复制 skills 目录，自动加载 `tsk@skills-dir`），见 README「二、安装 / 三、使用」。
- **开发者源码装插件**：`./install.sh --plugin`（Windows: `install.bat --plugin`）——重编后一并装到 `~/.claude/skills/tsk`。

### 6.6 一键卸载

release 包（`dist/tsk-v0.1.0.zip` 与 `dist/tsk-plugin-v0.1.0.zip`）内含 **uninstall.bat / uninstall.sh**：
删除插件目录 `~/.claude/skills/tsk`、`~/bin` 里的 tsk 二进制，并**清空统计** `~/.tsk`。

### 6.7 出包（生成两个分发 zip）

```bash
python tools/package.py
```

脚本会先 `cargo build --release`（保证最新），再生成：
- `dist/tsk-v0.1.0.zip` — 二进制包（顶层 `tsk.exe` + install/uninstall.bat·sh + README/PROJECT/LICENSE/NOTICE + examples）
- `dist/tsk-plugin-v0.1.0.zip` — Claude 插件包（插件文件 + install/uninstall 脚本）

包内 `install.bat` 双击即可用（自动 PATH + 结尾 pause 让结果可见）；`uninstall.bat` 一键卸载（含清统计）。

### 6.8 沙箱脚本约定

自动路由（P-1）需要项目预置分析脚本：`<project>/.tsk/sandbox/analyze_<ext>.{sh,py,js}`（ext ∈ log/jsonl/csv/json/tsv）。模板见 `examples/sandbox/`。缺脚本时自动回退骨架（不猜测、不现造脚本）。

## 7. 可优化项（能改什么 + 每项具体收益）

按改动风险分四档。**收益均已按实测数据换算**（token 口径 = ⌈bytes/4⌉，与全工程一致），每项标注"省多少、每样本"与依据，避免空泛。

### 7.1 无损微观批（改动小、纯收益，建议全做）

| # | 优化项 | 现状 | 具体改法 | 收益（量化 + 依据） |
|---|---|---|---|---|
| 1 | 省略标记缩短 | `[TSK omitted lines 6..197. Full file: Read bigfile.txt with offset/limit, or Grep <pattern> bigfile.txt]` ≈ **110 字符 ≈ 28 tok** | 改为 `[TSK omit 6..197; Read/Grep bigfile.txt]` ≈ 45 字符 | 每骨架文件 **省 ~16 tok**（110→45 字符 = 65 字符/4）。骨架文件每个只在 tool_result 出现一次，次数=压缩次数 |
| 2 | 骨架头缩短 | `[TSK SKELETON of bigfile.txt \| 200 lines total \| head 5 + tail 3 \| TSK v1]` ≈ 73 字符 ≈ 18 tok | 精简为约 40 字符，保留文件名、行数、版本关键字段 | 每骨架 **省 ~8 tok**（73→40 = 33/4） |
| 3 | 外置摘要 1KB→256B | 指针文件含 **1KB excerpt ≈ 256 tok** | 摘要降到 256B（日志/数据文件前几行足够识别格式） | 每次外置 **省 ~192 tok**（256−64）；全文仍落盘可回读，无损 |
| 4 | 骨架路径 hash 40→8 位 | 路径 `…/<sha1-40>-<name>.skeleton.txt` 约 70 字符 | hash 截 8 位（碰撞率 2^-32，可接受） | 每次骨架 Read **省 ~8 tok**（路径缩短 ~30 字符，出现于 updatedInput + 模型引用 1-2 次）。收益最小，优先级低 |
| 5 | AUTO-CLARITY 段精简 | ruleset 3867 字符含该段 ~200 字符 | 压缩安全例外措辞到 ~120 字符 | 每会话注入 **省 ~22 tok**（200−120=80/4，约 22）——比原估的 50 更准 |
| 6 | 行号列宽自适应 | 6 列右对齐固定，1 位行号也占 6 列 | 按最大行号位数定宽（≤9999 行时最多 4 列） | 小文件骨架每行省 2-5 字符，8 行合计 **省 8-40 tok**；**改动影响 §6.1 确定性格式，需重跑黄金测试**，性价比一般 |
| 7 | 180k 建议线 → ~110k | `COMPACT_ADVICE_THRESHOLD=180k`；实测平台 auto-compact 在 **~119k** 已触发 | 阈值降到 ~110k | **非直接 token 节省**：让 TSK 的 `/compact` 建议在平台自动压缩（~119k）之前给出，用户可主动管理上下文，避免平台静默压缩丢信息。这是"时机可控"，不是省 token 数 |
| ✅ | report est token 净值列 | — | 已完成（`tsk report` 输出 saved / inject cost / net 的 est tokens） | 见 §2.5 |

### 7.2 机制级（中等工作量，结构性收益）

| # | 优化项 | 现状 | 具体改法 | 收益（量化 + 依据） |
|---|---|---|---|---|
| 8 | P-1 沙箱路由扩展 | 只识别 {log,jsonl,csv,json,tsv}，且需 `<project>/.tsk/sandbox/analyze_<ext>` 脚本存在 | 补 `examples/sandbox/analyze_json.py`、`analyze_csv.py` 模板，推广到 json/csv 类数据文件 | **每档案外省 ~68 tok**：一个 100KB 聚合文件，骨架约进 context 280B(70 tok)，沙箱结论约 5B(≈1 tok)；依据实测：沙箱 -100%（2000 行 log→行数结论）vs 骨架约 -80%（70 tok 残留）。这是把"中等降到很低" |
| 9 | reinforce 按需跳过 | 每轮都注入 reinforce（est 88 tok） | 读 transcript 最后一条 assistant 回复，若"已明显在遵守压缩文风"（回复短、无客套）则本轮跳过 | 长会话中每跳一轮 **省 88 tok**；30 轮若 20 轮已遵守 → **省 ~1760 tok/会话**。依据：reinforce 实测 est 88 tok |
| 10 | UserPrompt 定期落快照 | 快照只在 PreCompact 写；实测平台 auto-compact **不触发**该钩子 → 现实中快照几乎从不更新 | 每 N 轮 UserPromptSubmit 增量重算 decisions/skills/intent 覆写 resume-snapshot.json | 非直接 token：auto-compact 后 `SessionStart(resume)` 能带回最新决策，**resume 后少 1-2 轮重新探索**（每次 resume 相当于省数百 tok 的重新读） |
| 11 | resume 注入用满预算 | 快照小 → assemble 只注入 ~49/500 tok | 把最近更多 decisions/skills 填满 ≤500 tok 预算 | 交易式：注入从 49→~300 tok（多花 ~63 tok），换来 resume 后更少的重探索轮数。不是纯省，是"多注入换少探索" |
| 12 | marker 提示批量取回 / Grep 优先 | marker 写 "Read with offset/limit, or Grep"；模型常一次只取 1 行 | 文案改为 "一次 `limit: N` 取多行"、"数据类用 Grep 优先" | **预计省取回工具开销 30-50%**：每次取回 ~90B（escape_retrieval_cost 实测）；30 次取回若减半 → 省 ~15×90B≈3.4KB≈**约 840 tok/会话**（取决于模型是否遵循，属行为引导，非硬保证） |
| 13 | 检测 auto-compact → 补 resume | auto-compact 不调 PreCompact hook，resume 注入拿不到新快照 | UserPrompt 时读 transcript 尾部，检测到 compact 痕迹则本轮补快照注入 | 非直接 token：在 auto-compact 路径上恢复上下文，效果同 #10。与 #10 二选一或合并实现 |

### 7.3 需实验（有损风险，先 A/B 再上）

| # | 优化项 | 现状 | 具体改法 | 收益 / 风险 |
|---|---|---|---|---|
| 14 | ruleset 压缩到 ~2500 字符（est 625） | full 现 3867 字符(est 967) | 裁剪正反例/自检，目标 est 625 | 收益：**break-even 从 3 轮提前到 2 轮**（若 est 625：净收益 = 560N −(625+88N)=472N−625，N=2 即 +319 tok），且每会话省约 **342 tok 注入**（967−625）。风险：2500 < S3 实测阈值 3500，压缩效果可能稀释，需 A/B 重测 §12.3 |
| 15 | ultra 档更激进出/文风 | ultra 与 full 同文本 | 加更紧凑规则（如单例、去横向字段） | 输出在 -68.8% 基础上再降 5-15%（未验证）；风险：电报体影响可读性 |
| 16 | reinforce 缩短 200→120 字符 | est 88 tok/轮 | 精简强化文本 | 每轮省 ~20 tok 注入；风险：约束弱化（S3+ 口径需重测） |
| 17 | 中文 ruleset | 英文 ruleset 对中文回复约束效率低 | 中文用户注入中文规则集 | 中文会话输出可能进一步省（未验证）；风险：AUTO-CLARITY 例外需同步中文化 |

### 7.4 平台受限（明确不做）

| # | 提升点 | 锁死原因 |
|---|---|---|
| 18 | Bash / Grep 大输出压缩 | PostToolUse 无法改写 tool_result（S2）；改写成管道/多语句触发二次权限弹窗（S1） |
| 19 | 行级过滤 / 摘要骨架 | 已实测失真（rtk #822），文档 §11.3 明确排除 |
| 20 | 输出长度硬上限 | auto-clarity 安全例外难覆盖，有截断正确信息的风险 |

**执行顺序建议**：
1. **7.1（#1-5）**：改动在常量的文本/路径层面，一次提交全部落地，每骨架省 ~30 tok、每外置省 ~192 tok，无损、低风险。
2. **7.2（#8-13 按序）**：#8（沙箱路由扩展，收益最直接）→ #9（reinforce 跳过）→ #10/#13（auto-compact 恢复）→ #12（行为引导）。
3. **7.3** 一律先做 A/B 实验（§12.3 复现法），通过再上；不通过保持现状。
4. 7.4 保持不做。

*TSK · Token Saving Kit · 58 测试全绿 · 真实会话验证通过*

---

## 8. DevEco Code 接入与 benchmark 结论（v0.1.0 + devdeco 适配）

### 8.1 另一接入方式：DevEco Code（opencode 系）

不在 Claude 插件范围内，单独文档落在 `integration/deveco/`：

- **插件本体** `integration/deveco/tsk.plugin.ts`（+ 常驻 `no-emulator.ts`），按 deveco 插件契约 `{ id, server({directory}) => Hooks }` 加载；
- **能力**：read 无损、Bash/build 大输出沙箱蒸馏、文件化知识库（`.tsk/ctx` + JSON 倒排索引）、SessionStart/compact 记账、**常驻禁止启动模拟器/设备/真机**；
- **接入说明 / 用法**：`integration/deveco/README.md`、`integration/deveco/DEVECO_USAGE.md`。

### 8.2 与 Claude 核心的关系

deveco 插件**复用同一 Rust 核心**（`tsk hook PreToolUse/PostToolUse/SessionStart`），但不注入 Claude 那套会话规则集（deveco 的 `experimental.chat.messages.transform` 在现装二进制上只给 ≤7 消息的有界窗口，历史压缩不可行）。

### 8.3 benchmark 实测结论（模型 = GLM-5.1 / deepseek-v4-flash，bootstrap 单 case）

| 杠杆 | 结论 | 证据 |
|---|---|---|
| 输入压缩（read 改写） | **在 GLM-5.1 上会破 case**（GLM 拿到摘要即 0 文件不建 / 确定性 ~76.8k 早退） | 4 个不同实现一致在 ~76.8k fail |
| 输入压缩（deepseek） | 骨架化直接导致建不对（file_missing） | deepseek 早期实验 |
| 输出沙箱（L1） | code-gen 输出小，节省仅 ≈0.1% | 3 case 省 1.6k tok / ~1.4M total |
| 历史压缩（L2） | deveco transform 只给 ≤7 有界窗口，无可裁剪历史 | msgs 序列 1..7 循环 |
| 成功率 | **输入无损 + 输出沙箱：on pass = off pass（smoke-3 3/3）** | 已跑通 |
| 模拟器 | 插件 + 看门狗双保险，on/off 都拦截 | 已封 |

**一句话**：在这台机 + deveco 0.1.12 + GLM-5.1 + code-gen bootstrap 上，TSK 能**保成功率**，但 token 省结构性受限——
输入压缩会破模型建码、输出太小、历史窗口有界。要拿真实收益需换模型/换大输出密集 workload（超出 TSK 插件层），详见 `integration/deveco/benchmark/` 各报告与本文 §7。

### 8.4 已并入主干的 Rust 改动

- `core/src/inject.rs`：沙箱脚本支持**全局目录**（env `TSK_SANDBOX_DIR`），项目脚本缺失可从全局找，避免在模型可见工作区放 TSK 产物。
- 其余为 deveco 插件侧（`integration/deveco/`），核心零侵入。
