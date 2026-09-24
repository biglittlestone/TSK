# TSK × DevEco Code 接入与使用指南

本文档说明如何把 TSK 接进 **DevEco Code**（基于 opencode 的编程 agent），以及各能力怎么用。
DevEco 没有 Claude Code 式的 shell hooks，改用纯 JS/TS 插件（`deveco.json` + `.deveco/plugin/` 自动发现）。
插件把 TSK 的输入压缩 / 输出压缩 / 历史快照接进来，**复用同一个 Rust 核心**，对 DevEco 零侵入、全程 fail-open。

---

## 1. 前提

- `tsk` 已安装且能访问（`tsk --version` 输出 `tsk 0.1.0` 即通过）。
  - 没装？见 TSK 主 README 的安装节（`install.sh` / `install.bat` 装到 `~/bin`/`%USERPROFILE%\bin` 并加 PATH）。
- `tsk --version` 能跑到，说明 DevEco 进程能通过 PATH 找到 `tsk`（插件用 `Bun.$` 调裸名 `tsk`）。

## 2. 安装

把插件文件 `integration/deveco/tsk.plugin.ts` 放到 DevEco 的插件自动发现目录（二选一）：

```bash
# 全局（所有 DevEco 项目）
mkdir -p ~/.deveco/plugin
cp integration/deveco/tsk.plugin.ts ~/.deveco/plugin/tsk.ts

# Windows 等价路径
mkdir -p %USERPROFILE%\.deveco\plugin
copy integration\deveco\tsk.plugin.ts %USERPROFILE%\.deveco\plugin\tsk.ts

# 或只对单个项目
# 把 tsk.plugin.ts 拷到 <project>/.deveco/plugin/tsk.ts
```

DevEco 启动时自动加载 `.deveco/plugin/*.ts`，**无需**改 `deveco.json`。

## 3. 配置 TSK（复用同一份 `~/.tsk/config.yaml`）

TSK 的配置在 `~/.tsk/config.yaml`，DevEco 侧与 Claude 侧共用，其中影响 DevEco 的开关是**输出压缩档位**：

```yaml
enabled: true
output_compression: off   # off | lite | full | ultra | auto
```

| 档位 | DevEco 行为 |
|---|---|
| `off`（默认） | 不注入任何输出压缩规则；输入压缩仍在 |
| `lite` | 注入精简 ruleset（est ~361 tok/会话），较弱文风压缩 |
| `full` | 注入完整 ruleset（est ~967 tok），最强文风压缩（实测输出 -68.8%） |
| `ultra` | 同 `full`（预留） |
| `auto` | 前 2 轮零注入；第 3 轮起注入 ruleset（每轮 reinforce）——按会话长度自动开关 |

> 不改也行：`off` 下输入压缩照常，只是回复不自动变简洁。
> 要启用输出压缩，编辑 `~/.tsk/config.yaml` 把 `output_compression` 换成目标档位即可。

## 4. 验证

1. `tsk --version` 确认二进制可用。
2. 新开一个 DevEco 会话，让模型 `read` 一个大文件（≥8 行、>1KB）。
3. 应看到它实际读的是 `.tsk/skeletons/<sha1>-<name>.skeleton.txt`（骨架）。
4. 模型要取中间行时用 offset/limit 回读原文（逃逸口无损）。
5. （若开了 full/lite/auto）看回复文风是否变简洁。

无 DevEco 运行时也可在沙盒自测，见 §7。

## 5. 能力清单与使用

### 5.1 输入压缩（骨架 / 外置 / 逃逸口）

- **骨架化**：模型 `read` 大文件时，`filePath` 被改写为 `.tsk/skeletons/<sha1>-<name>.skeleton.txt`（head/tail 摘要），原文不进上下文。
- **外置**：>100KB 文件改写为 `ext/<hash>.txt` 指针，模型按 `full:` 路径按需回读，不经压缩。
- **逃逸口**：带 `offset`/`limit` 的 `read` 恒放行，无损取回原文。
- 全程 fail-open：任何失败 Read 原样执行。

### 5.2 输出压缩

按 §3 的档位注入 system prompt；注入幂等（system 已有 TSK 标记则不重复）。

- `off` / `lite` / `full` / `ultra`：会话创建时一次性注入 ruleset；`full` 起每轮追加 reinforce。
- `auto`：短会话（≤2 轮）零成本，第 3 轮起注入。

### 5.3 历史快照（compact → resume）

会话被 compact 时，插件落一份 resume-snapshot，并在 compact 后把 ROLE/DECISIONS/SKILLS/INTENT 注入回去，减少 compact 后的重新探索。

---

## 6. 可选环境变量

DevEco 是 opencode fork，两类环境绑定点的真实事件/路径名因 fork 而异，二者都**可省略**——省略时的行为见各自说明：

| 变量 | 作用 | 省略时 |
|---|---|---|
| `TSK_DEVECO_TURN_EVENTS` | 逗号分隔、应视作"新用户消息轮"的 `event.type` 值；auto 档靠它计数。缺省 `chat.partial,message.updated,message.user,user.message` | auto 档观测不到用户轮则不激活（仍零成本，其余档位不受影响） |
| `TSK_DEVECO_TRANSCRIPT` | compact 时 snapshot 用的 transcript 路径；优先从 `session.compacted` 事件的 `transcriptPath`/`properties.transcriptPath` 取 | 拿不到 transcript 则跳过 compact 快照（resume 无内容，fail-open） |

设置方式：在启动 DevEco 的进程环境里 export（Windows 用 `set`）。如果你的 fork 事件名不同，改这个变量即可，无需改代码。

## 7. 沙盒自测（无需 DevEco 运行时）

```bash
node integration/deveco/smoke.test.mjs   # 需 tsk 在 PATH；或设 TSK_BIN=/path/to/tsk
```

用**真实 `tsk` 二进制 + 真实插件**驱动模拟事件流，数据落隔离临时 `TSK_HOME`。覆盖 17 项：
骨架改写、逃逸口 passthrough、非 read 不改写、full/lite/off 注入与幂等、auto 轮 1/2 不注 / 轮 3 注入 / 轮 4 reinforce / ruleset 恰一次、compact→RESUME + 锚点 + 快照落盘。

## 8. 卸载

```bash
rm -f ~/.deveco/plugin/tsk.ts        # 或项目 .deveco/plugin/tsk.ts
```

## 9. 已知限制

- 沙盒已覆盖**逻辑正确性**；真实 DevEco 实测需在有 DevEco Code 运行时的机器上做。
- auto 的"每用户消息轮"观测点与 compact 的 transcript 定位为**环境绑定**（§6），用真实事件后按需调 `TSK_DEVECO_TURN_EVENTS`。
- 插件在 DevEco 的 Bun 环境运行，需确保 `tsk` 在 PATH。