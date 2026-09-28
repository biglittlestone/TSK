# TSK 端到端基准 —— 实测结果报告

> 日期：2026-09-24 · 全部用真实模型（opus-4-8 via local proxy）+ 真实多轮 `claude -p`。
> cost 为代理侧计费的美元；input = `input+cache_read` 累计。TSK=Claude-native 插件（Read-hook）。

## 已跑用例（4 场，均 PASS；两场 ON 命中 30 轮上限被截断，但产物已自验通过）

| 场景 | 模式 | 轮数 | 总输入 tok | 成本$ | TSK 动作 | 结论 |
|---|---|---|---|---|---|---|
| 01-calc-web（小 spec→网页计算器） | ON | 31(截断) | 978K | 4.90 | Read 骨架 1.4KB spec + 逃逸口整读回 | 净省 0 |
| 02-library-crud（大 288KB 数据集→CRUD） | ON | 31(截断) | 1224K | 6.43 | 无——模型用 bash/node 抽样，从不 Read | 净省 74B |
| 04-sandbox-doc（197KB 文档→概览） | ON | 24(完成) | 628K | 3.12 | `tsk analyze/search/exec` 沙箱全程，0 次 Read | 正确路由到沙箱，未共享主导 |
| 04-sandbox-doc（同任务，OFF 无 tsk） | OFF | 20(完成) | 480K | 2.31 | 无 | 反而更低（见注） |

## 三个决定性发现

1. **Read 骨架被「逃逸口整读」抵消（01）**。TSK 把 spec 骨架化“省 809B”，模型随即经 offset/limit 把它整读回 → `tsk report` 净省 0（悲观上界甚至为负）。凡“必须看懂全文”的活，骨架不省。

2. **模型改用 Bash/Node 消费大数据，绕开 Read 钩子（02）**。288KB 数据集进 02 会话，但会话里 0 次 `Read`、0 次命中 books.json——模型用 `node -e require()/抽样` 读取。TSK 的 Claude-native 插件**只在 Read 前后挂钩**，读不到 Bash 路径。

3. **沙箱执行 ≈ 模型自己的定向 Bash（04）**。ON 全走沙箱且正确完成；但提取类任务里，模型用 `grep/head` 也能拿到同样小输出。OFF 那个“更低”数字是被污染基线：task.md 命令它用 tsk，而 OFF 没有 tsk，它烧掉约 10 轮调用不存在的 `tsk analyze`（空/失败输出不涨上下文，所以便宜靠的是失败，不是省）。

## 根因

Claude-native TSK 唯一被挂钩的收益通道是 **Read 前置重写**。它真正的两个杠杆在 Claude Code 上都失效：
- **Read 骨架**：逃逸口让模型可无损整读回 → 净零；
- **Bash/大输出压缩**：只存在 deveco/opencode 插件里（`tool.execute.after` 重写工具输出），Claude 平台**没有**这个钩子的对等实现。

## 可行动结论

要让 TSK 在 Claude Code 上真正省 token，缺口不是用例设计、而是**压缩“模型收到的工具结果”**本身（Read 与 Bash 的 PostToolUse）——即镜像 deveco 的 `tool.execute.after`：把模型将要收到的大输出骨架/蒸馏成 5+尾行，而不是只在 Read 请求侧做、且留下可整读的逃逸口。这属于 TSK 代码改动（新方向），成本可再生，收益才有可能 >0。

## 附：脚本与复现
- `scenarios/run-onoff.sh <scenario> <out> [on|off]`：单场景 on/off 跑手。OFF 卸插件 + 从 PATH 摘 tsk（保证无沙箱二进制）；用 `--output-format stream-json --verbose` 留全程（含工具调用）。末尾 `trap` 恢复 TSK。
- `scenarios/measure.mjs <cap> [--tools]`：解析 aggregated 或 stream-json，累计真实 input tok，`--tools` 列工具调用。
- 收益口径：`tsk report --session <id>` 给 ledger（骨架/回读/净省）——回读已计入，故 ON 的 ledger 净省为主导证据；input-tok 累计为总口径。

## 待办
- [ ] 决定是否实现 Claude-native 的 **工具结果压缩**（PostToolUse 对 Read/Bash 输出做骨架/蒸馏）——那是能真正产生>0收益的方向，而非再堆用例。
- [ ] 03-copy-adapt 未跑（设计上最贴近“复制整引用”，预期仍回读抵消）。