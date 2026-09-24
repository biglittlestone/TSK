# TSK — Token Saving Kit

Rust 核心 + Claude Code hooks 的 token 节省插件。三条压缩线（输入 / 输出 / 历史），
装入后**完全由钩子自动运行，agent 侧无感知**——不需要敲任何"压缩 token"的命令。

> - 本文是**终端用户指南**（装插件 + 使用）。实现规格见 [`info.md`](./info.md)，技术手册见 [`PROJECT.md`](./PROJECT.md)。
> - 从**源码**安装非插件版（开发/自定义构建）：见 [`PROJECT.md §6`](./PROJECT.md)。本文只讲 release 插件包。

---

## 一、它是什么

TSK（Token Saving Kit）：Rust 核心 + 钩子的 **token 节省工具**。它通过**插件方式**接入 agent，
在输入压缩 / 输出压缩 / 历史压缩三条线上自动省 token，agent 侧无感知、全程 fail-open。

> **本 README 只讲 Claude Code 接入方式。** 其他接入方式（如 DevEco Code）见独立文档
> **[接入方式·DevEco Code](./integration/deveco/README.md)**。

| 压缩线 | 机制 | 默认 | 正确性 |
|---|---|---|---|
| 输入 | 读大文件自动**骨架化**（留首尾）/ **外置**（>100KB 全文落盘）/ 沙箱分析 | 开 | 无损（原文可经逃逸口回读） |
| 输出 | 注入文风规则，让回答更简洁 | 关（需开启） | 无损（代码/命令逐字，仅文风变） |
| 历史 | 压缩前**快照**，恢复时自动带回上下文 | 开 | 无损 |

**实测收益速览**（每项对应输入规模见「四、效果与收益」）：
骨架 **-83%** ｜ 外置 **-99.4%** ｜ 沙箱分析 **≈-100%** ｜ 输出压缩 **-68.8%** ｜ 30 轮长会话 **-99.2%**

---

## 二、安装（插件，release 包）

> 最省事的安装方式：装成 Claude 插件，hooks 与斜杠命令全部自动。需要从源码构建/自定义安装？见 `PROJECT.md §6`。

```bash
# ① 获取并解压插件包 dist/tsk-plugin-v0.1.0.zip
# ② 先解压到文件夹（不要在压缩包内双击），再运行包内安装脚本：
#    自动把 tsk 放进 ~/bin + 加入 PATH；把插件放进 ~/.claude/skills/tsk
#    Windows     : install.bat
#    macOS/Linux : sh install.sh
# ③ 新会话自动加载（tsk@skills-dir），即可使用
```

脚本两个动作，都不需要你手动改 PATH / 环境变量：
1. 把 `tsk` 二进制装到 `~/bin`（插件钩子命令按裸名 `tsk` 解析，需它在 PATH）；
2. 把插件复制到 `~/.claude/skills/tsk/`。

### 验证

```bash
tsk --version                    # tsk 0.1.0
claude plugin list               # 应见 tsk@skills-dir，Status ✔ loaded
claude plugin details tsk@skills-dir
```

> 若 `claude plugin list` 显示 `Status: × disabled`（例如曾在 `/plugin` 里 disable 过、disable 状态被记录），安装脚本已自动执行过 `claude plugin enable tsk@skills-dir`；仍 disabled 就手动运行一次：`claude plugin enable tsk@skills-dir`。

新会话里 `Read` 一个大文件，Claude 收到的应是 `[TSK SKELETON of ...]`（钩子生效）。

---

## 三、使用

**没得用。** 装上之后 hooks 就在跑。

### 斜杠命令

| 命令 | 作用 |
|---|---|
| `/tsk-report` | **当前会话** token 节省报告（saved / 注入成本 / 净值） |
| `/tsk-report-all` | 全量（累计所有会话）统计 |
| `/tsk-report-clean` | 清空统计（删 ledger + 当前标记，**不可逆**） |
| `/tsk-doctor` | 自检：配置 / 可写 / 钩子接线 / PATH |
| `/tsk-exec <子命令>` | 运行任意 `tsk` 子命令（如 `/tsk-exec off` 一键关闭） |

### 唯一配置：`output_compression`

编辑 `~/.tsk/config.yaml`。这是**唯一改变回答文风（用户可感知）的选项**——它让回答更简洁，
但技术正确性始终无损（代码/命令/路径逐字、安全例外 AUTO-CLARITY）。

```yaml
enabled: true            # 一键全关（排查安全阀）
output_compression: auto # off | lite | full | ultra | auto —— 唯一真正的用户决策
```

| 档位 | 说明 | 注入成本 | 效果 |
|---|---|---|---|
| `off` | 不压缩输出（默认） | 0 | 无 |
| `lite` | 短规则集 | est 361 tok/会话 | 较弱 |
| `full` | 完整规则集 | est 967 tok/会话 | 最强（真实 A/B -68.8%） |
| `ultra` | 同 full（预留更激进档） | est 967 tok/会话 | 同 full |
| `auto` | 按会话轮数启停（≤2 轮关、≥3 轮开） | 短会话 0 | 长会话同 full |

### 卸载

包内已含**一键卸载脚本**：解压 release 包后运行 `uninstall.bat`（Windows）或 `sh uninstall.sh`（macOS/Linux）。它删除：
- 插件目录 `~/.claude/skills/tsk`
- `~/bin` 里的 tsk 二进制
- **并清空统计** `~/.tsk`（卸载即彻底）

如需逐条手动卸载：

```bash
claude plugin disable tsk@skills-dir   # 停用（可再 enable）
rm -rf ~/.claude/skills/tsk            # 卸载插件
rm -f ~/bin/tsk*                        # 移除二进制
rm -rf ~/.tsk                           # （可选）清空统计
```

### 疑难

| 现象 | 处理 |
|---|---|
| `claude plugin list` 未见 `tsk@skills-dir` | 确认目录在 `~/.claude/skills/tsk/`；新会话才加载（当前会话用 `/tsk-exec` 或重启） |
| Read 大文件没变成骨架 | `/tsk-doctor`：检查钩子接线 + `tsk in PATH` |
| `/tsk-report` 无数据 | 当前会话还没产生台账（读过大文件才有）；或已被 clean 清空 |
| 卸载后 hooks 仍在 | 新会话才干净；已开的会话可能缓存旧 hooks |

---

## 四、效果与收益

### 收益场景口径（先读，避免误解百分比）

所有收益都是**先给定输入规模、再报结果**的实测/估算。例如「骨架 -83.3%」指**一个 200 行文件**；
「外置 -99.4%」指**一个 4000 行/201KB 日志**；「输出 -68.8%」指**单轮 prompt "explain closures"**。
收益随输入规模、文件类型、模型、任务变化，同规模才可比。token 口径统一 est_tokens = ⌈bytes/4⌉。

### 收益口径（悲观上限 + 取回成本）

`tsk report` 的 saved、本 README 的收益表，都是**纯机制节省**（压缩产物 vs 全文的字节差），
**默认假设"模型只需骨架/摘要、不回读全文"**。但压缩会因为"模型之后要不要读原文全文"而有边界：

| 场景 | 成本 |
|---|---|
| 只要摘要：读骨架 | est 73 tok |
| 要中间 2 行：骨架 + 2 次取回 | est 118 tok |
| 要全文：骨架 + 5 次取回 | est 635 tok（直读全文只要 423）→ **额外 +212 tok (+50%)** |

`tsk report` 末尾会直接给出**悲观上限**：被压缩原文合计 + 「若模型把原文全部回读」时的额外输入与净省
（回读时净省**趋近或为负**）——把这个回读成本如实摆在报告里，不隐藏。**真实收益落在"乐观 saved"与"悲观上限"之间**，取决于模型实际取回多少（`retrievals` 行显示真实取回次数）。

结论：**压缩省 token 的前提是模型不需要回读全文**；`report` 的 saved 是"模型按提示高效使用骨架/摘要"情景下的节省。

### 三条压缩线实测

| 机制 | 场景 | 原来 | 压缩后 | 收益 |
|---|---|---|---|---|
| 骨架 | 200 行文件 | est 423 tok | est 71 tok | **-83.3%** |
| 外置 | 4000 行日志 201KB | est 50447 tok | est 319 tok | **-99.4%** |
| 沙箱分析 | 500 行日志 | est 11946 tok | est 6 tok | **≈-100%** |
| 多轮 30 轮 | 混合任务 | est 408720 tok | est 3126 tok | **-99.2%** |
| 真实工程 30 轮 | 复制工程定位/修 bug | est 41958 tok | est 3072 tok | **-92.9%** |

### 输出压缩（真实 claude A/B，单轮 prompt "explain closures"）

| 档位 | output_tokens | 文本 | 开头 |
|---|---|---|---|
| `off` | 814 | 2753 字符 | `# Closures Explained`（长文） |
| `full` | 254 | 728 字符 | `Closure = function retaining access…`（电报体） |

**输出 -68.8%**。代价：注入 ruleset est 967 + 每轮 reinforce est 88 tok；**≤2 轮净亏、≥3 轮净赚**（`auto` 档自动处理）。

### 真实 30 轮长会话（full 输出压缩 vs off）

| 指标 | OFF | ON | 差异 |
|---|---|---|---|
| output_tokens | 35,621 | 8,732 | **-75.5%** |
| 输出文本 | 2848B | 1552B | -45.5% |
| 输入（含 cache） | 908,614 | 309,418 | **-66%** |
| cache_read 占比 | 76.2% | 95.6% | 注入稳定反提高命中 |

漂移检查：ON 会话最后回复仍为电报体列表（无漂回长文）。

---

## 五、测试

58 项自动化 = 17 单元 + 10 集成 + 31 冒烟，全绿、0 warning。技术手册 `PROJECT.md §5` 有完整矩阵与量化数据。

```bash
cargo test                                # 58 项全绿
cargo test --test smoke -- --nocapture    # 冒烟带量化输出
python tools/smoke_table.py               # 冒烟执行结果表格
```

**测能力不测覆盖**：每个测试一个功能点 + 真实字节断言；CI 守护确定性逐字节、fail-open 恒 `{}`、无损取回 == 原文、并发台账 0 坏行、收益阈值。

## 六、设计原则

1. **代码极简**。依赖锁在 `serde / serde_json / serde_yaml / sha1 / clap`（测试用 `assert_cmd`）。
2. **完整测试集，测能力不测覆盖**。
3. **使用方式极简**。用户只有 `init` 一次 + 一个可选配置项。
4. **一键关闭**。`tsk off` 幂等，关得干净。
5. **用户无感知**。压缩全走 hooks。

## 七、已知限制（v1）

- **Bash / Grep 大输出外置**：平台无法改写工具结果，v1 无解；Bash 工具事件恒放行（防二次权限弹窗）。
- **网络隔离**：Linux `TSK_NET_ISOLATE=1` 可用（`unshare -n`，需 CAP_SYS_ADMIN）；Windows 不支持。
- 依赖 `dlltool` 的平台已绕开 `windows-sys`，正常 Rust 工具链可直接构建。

## 八、许可

Apache-2.0。见 `LICENSE` / `NOTICE`。