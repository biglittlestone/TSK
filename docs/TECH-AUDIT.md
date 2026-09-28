# TSK 技术点梳理与无效代码清查

> 依据：本会话 6 场真实 on/off + 多次对拍 + rtk/context-mode 源码研读，逐项实证。
> 结论先行：**输出压缩（rtk 式逐命令语义压缩 + KB/BM25 回读）是唯一真收益，保留**；其余多为无收益的输入/文风/历史装置，列为待清或已停。

## 一、总览——TSK 的两块集成面

| 面 | 位置 | 作用 |
|---|---|---|
| Claude Code 原生 | `~/.claude/skills/tsk` + Rust `core/cli` | hook 插件 + MCP 沙箱工具 |
| opencode/deveco | `integration/deveco/tsk.plugin.ts` | tool.execute.after 输出压缩 + KB |

## 二、技术点清单 + 收益判定（◈=保留 ◇=视情况 ◇✗=待清）

### 真收益（保留 ◈）
| 技术点 | 位置 | 证据 & 说明 |
|---|---|---|
| **rtk 式逐命令输出压缩**（ls/git status/log/diff/find/grep/wc/du/df/ps/test） | deveco `tool.execute.after` + `compactByCommand` | 真实大输出 80-97% 削减（rtk-benefit-test）；格式保持、无损落盘 |
| **KB 落盘 + B 分节地图 + tsk search(BM25)+C 高亮** | deveco KB + `core/search.rs` | 「统一回读走 BM25」；分节索引、按节精确回取；无损 |
| **套娃守卫**（读自身 .tsk/kb 直通） | deveco after | 第二次读原文不再二次压缩 |
| **NO_EMULATOR / curl 拦截** | deveco before | 行为策略（防失控），非 token 但必要 |
| **会话 compact→resume** | deveco event / claude PreCompact+SessionStart | 长会话连续性（防上下文丢失），非直接省但也必要 |

### 视情况（保留 ◇）
| 技术点 | 位置 | 说明 |
|---|---|---|
| **Claude MCP 沙箱工具**（tsk_analyze/search/exec） | dist/mcp + plugin.json mcpServers | 整文件分析任务上真省、per-call 保证；但强模型会用 grep 绕过 → 只在"分析型"任务显收益 |
| **tsk report 记账** | Rust | 目前只记 Read 骨架（≈0）；需改口径去记输出压缩才仍有审计价值 |

### 无收益 / 已停（待清 ◇✗）
| 技术点 | 位置 | 判定依据 |
|---|---|---|
| **Read 骨架输入压缩**（PreToolUse Read） | claude 插件 + Rust pretooluse | 逃逸口(offset/limit)把骨架整读回 → net 0（01 对拍） |
| **PostToolUse(Read)** | claude 插件 + Rust posttooluse | 纯 noop/log，无收益 |
| **输出文风 ruleset 注入**（UserPromptSubmit/SessionStart） | claude + Rust | 你已 `output_compression: off`；实测是净开销（注入 ~967tok vs 强模型几乎无省）→ 建议彻底移除该装置 |
| **deveco L2 messages.transform** | deveco plugin | `if(true) return` 死分支；且时序上先 raw 后回改会破 cache（G 结论）→ 建议删 |
| **outputSummaries 输出改写**（若有残留） | claude | 实测不喂模型（marker 探针"无"）→ 无效 |
| **M2/BENCHMARK 里的沙箱 on/off 期望** | 仅文档 | 强模型端到端 ON≥OFF（对拍），非插件收益点 |

## 三、明确保留：输出压缩（你强调的）
- deveco `tool.execute.after` 的 rtk 式输出压缩 + B/C/KB/search —— **核心收益，动不得**。
- 「输出文风(css ruleset)」是另一回事（已 off，且无收益），与「输出压缩」区分开，别混。

## 四、建议的清理动作（等你拍板，我不擅删）
1. 删 claude 插件 PreToolUse(Read) + PostToolUse(Read) hook（与 Rust pretooluse/posttooluse）—— 输入骨架 net-0、post noop。
2. 删 userPromptSubmit/SessionStart 的 文风 ruleset 注入路径（config 已 off；连装置一起清）。
3. 删 deveco `experimental.chat.messages.transform` 死分支。
4. `tsk report` 记账口径改记「输出压缩 saved」（原始-保持），替代现在的 read 骨架口径。

> 保守项不删：MCP 沙箱（◇）、deveco 的 before 策略与 compact/resume（保连续性）、套娃守卫。
## 执行结果（本轮已落）
- **#1 插件层禁用 read 骨架**：plugin.json 与 `add_tsk_hooks` 移除 PreToolUse/PostToolUse hook（Claude 不再装）。**Rust pretooluse/posttooluse 实现保留**（取消物理删除——删它波及 14 个 smoke/e2e 断言骨架的测试；保留为未布线实现，测试仍绿）。
- **#2 文风路径保留**（config 仍 off）。
- **#3 拆除**：deveco L2 `experimental.chat.messages.transform` 死分支已删，smoke 39/39。
- **#4 口径**：`tsk report --all` 新增"deveco 输出压缩 saved token/N 次大输出"（读 `<cwd>/.tsk/injections.jsonl` sandbox-output）。
- Rust 测试 10+34+20 全绿 + deveco smoke 39/39；二进制已重部署。
