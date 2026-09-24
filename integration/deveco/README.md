# TSK × DevEco Code 适配

DevEco Code（基于 opencode 的编程 agent）**没有 Claude Code 式的 shell hooks**——它的 hooks 是
纯 JS/TS 插件（`deveco.json` 配置 + `.deveco/plugin/` 自动发现）。本目录提供 TSK 的适配插件，
把 TSK 的**输入压缩（骨架/外置/逃逸口）**接进 DevEco，复用同一个 Rust 核心。

## 支持能力

| 能力 | 插件钩子 | 效果 |
|---|---|---|
| 输入压缩（骨架化/外置 >100KB） | `tool.execute.before` + `read` | 模型 Read 大文件时，把 filePath 改写为骨架/外置指针 |
| 逃逸口（offset/limit） | 同上 | 带 offset/limit 的 Read 恒放行（无损取回原文） |
| 输出压缩（ruleset） | `session.created` + `experimental.chat.system.transform` | 会话创建时取 ruleset（full/lite/ultra 注入；off 不注）并注入 system prompt，幂等 |
| auto 档轮数化 | 逐轮 `tsk hook UserPromptSubmit` | 每个用户消息轮喂给 Rust 裁决：第 3 轮起注入 ruleset 一次 + 每轮 reinforce，≤2 轮零注入（与 Claude 侧 break-even≈3 一致）。观测点用 `TSK_DEVECO_TURN_EVENTS` 配置 |
| 历史/快照（compact→resume） | `session.compacted` + `PreCompact` + `SessionStart(source:compact)` | compact 时落 resume-snapshot、取 ROLE/DECISIONS/SKILLS/INTENT，compact 后注入回 system。transcript 路径用 `TSK_DEVECO_TRANSCRIPT` 或事件给出 |

插件本身很薄：`tool.execute.before` 把 DevEco 的 `read` 调用构造成 TSK 认识的 Claude 格式事件，
`Bun.$` 调 `tsk hook PreToolUse`，再把返回的骨架路径写回 `output.args.filePath`。
输出压缩与历史快照全部委托给 Rust 核心 `tsk hook` 裁决，插件只搬运 returned 的
`additionalContext` 冲入 system。**任何失败都不改写、不注入**（fail-open）。

> 环境变量（可选，两种"轮观测点"与 transcript 定位在真实 DevEco 上因 fork 而异）：
> - `TSK_DEVECO_TURN_EVENTS`：逗号分隔的、应视作"新用户消息轮"的 `event.type` 值。
>   缺省 `chat.partial,message.updated,message.user,user.message`。auto 档靠它计数；
>   观测不到用户轮时 auto 不激活（与改写前一致，仍零成本）。
> - `TSK_DEVECO_TRANSCRIPT`：compact 时 snapshot 用的 transcript 路径；优先从
>   `session.compacted` 事件的 `transcriptPath`/`properties.transcriptPath` 取，
>   都拿不到则跳过快照（resume 无内容可注入，fail-open）。

## 安装

**前提**：`tsk` 在 PATH（`tsk --version` 能过即可）。

把 `tsk.plugin.ts` 放到 DevEco 的插件自动发现目录（二选一）：

```bash
# 全局（所有 DevEco 项目）
mkdir -p ~/.deveco/plugin
cp integration/deveco/tsk.plugin.ts ~/.deveco/plugin/tsk.ts
# Windows 等价路径：%USERPROFILE%\.deveco\plugin\tsk.ts

# 或单项目：<project>/.deveco/plugin/tsk.ts
```

DevEco 启动时会自动加载 `.deveco/plugin/*.ts`；无需改 `deveco.json`。

## 冒烟测试（沙盒，无需 DevEco 运行时）

```bash
node integration/deveco/smoke.test.mjs
```

用真实 `tsk` 二进制 + 真实插件驱动 DevEco 事件流（session.created / tool.execute.before / system.transform），
数据落在隔离的临时 `TSK_HOME`。覆盖：骨架改写、逃逸口 passthrough、非 read 不改写、输出压缩 full/lite 注入与 off 不注、注入幂等。

## 验证

1. `tsk --version` 确认二进制可用；
2. 新开一个 DevEco 会话，让模型 `read` 一个大文件（≥8 行、>1KB）；
3. 应看到它实际读的是 `.tsk/skeletons/<sha1>-<name>.skeleton.txt`（骨架）；
4. 模型要取中间行时会用 offset/limit 回读原文（逃逸口无损）。

## 卸载

```bash
rm -f ~/.deveco/plugin/tsk.ts        # 或项目 .deveco/plugin/tsk.ts
```

## 已知限制

- 沙盒冒烟用真实 `tsk` 二进制 + 真实插件驱动**模拟的** DevEco 事件流验证逻辑正确性
  （骨架改写、逃逸口、full/lite/off/auto 注入、auto 第 3 轮 gate、compact→resume）。
  插件运行在 DevEco 的 Bun 环境，调用 `tsk` 用 `Bun.$`，需确保 `tsk` 在 DevEco 能访问的 PATH。
- **auto 档的"每用户消息轮"观测点**依赖 `TSK_DEVECO_TURN_EVENTS` 匹配的真实事件（因
  fork 而异，未在本机 DevEco 实测）。观测不到用户轮时 auto 不激活，其余档位不受影响。
- **compact→resume 的 transcript 定位**依赖 `TSK_DEVECO_TRANSCRIPT` 或事件携带的路径；
  真实 DevEco 上若拿不到 transcript，快照不落、resume 不注入（fail-open）。
- 以上两项为环境绑定，沙盒已覆盖逻辑正确性；真实 DevEco 实测需在有 DevEco Code 运行时的
  机器上做（对照悬在"无运行时"）。
