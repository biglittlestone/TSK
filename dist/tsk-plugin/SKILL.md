---
name: tsk
description: Token Saving Kit — 自动压缩输入/输出/历史，降低 Claude Code 会话的 token 消耗。安装后无需手动操作，钩子自动工作。
---

# TSK (Token Saving Kit)

TSK 在后台自动降低本会话的 token 消耗，**无需用户或模型做任何额外操作**——压缩由钩子（Hooks）自动完成：

- **输入压缩**：读取大文件时自动骨架化（只保留首 5 行 + 尾 3 行，中间行可通过 `offset`/`limit` 或 `Grep` 无损取回）；超大文件（>100KB）外置到磁盘，只送 1KB 摘要 + 指针。
- **输出压缩**：注入文风规则，让回答去客套、碎片化；代码/命令/路径逐字保留。（需在 `~/.tsk/config.yaml` 把 `output_compression` 设为 `lite`/`full`/`auto`，默认 `off`。）
- **历史压缩**：上下文压缩时保存快照，会话恢复时自动带回关键上下文。

## 何时需要用户注意

- 读了骨架后，如需中间某行，用 `Read` 的 `offset`/`limit` 参数取回（这是无损的逃逸口）。
- 想确认压缩是否生效：运行 `/tsk-report` 看 token 节省。
- 想关闭 TSK 或自检：用 `/tsk-exec off` 或 `/tsk-doctor`。

## 可用的斜杠命令

- `/tsk-report` — 当前会话的 token 节省报告（saved / 注入成本 / 净值，est tokens 预览）
- `/tsk-report-all` — 全量（累计所有会话）节省统计
- `/tsk-report-clean` — 清空 token 统计（不可逆，先确认）
- `/tsk-doctor` — 运行 TSK 自检（配置/可写/钩子接线/PATH）
- `/tsk-exec <子命令>` — 运行任意 `tsk` 子命令（如 `/tsk-exec off` 一键关闭）
## Think in Code — 面向大数据文件的省 token 方式（无损）

当面对**大文件/大输出**时，优先「算答案、别读全文」：

- **要汇总结果**（日志/测试/CSV/构建/错误数）：用 `/tsk-analyze <文件>` 在沙箱里分析，只把计算结果放进上下文；原始数据留在磁盘。
- **要精确原文/代码示例/API 签名**：用 `/tsk-search <词>` 在已索引的全文里按需取回**精确行窗**（代码块原样保留），不把整个文件读进上下文。
- **普通小文件**仍可 `Read`（无损）。

思想与 context-mode 一致：**不把 bulk 读进上下文，而是要什么就按需取什么（计算结果或精确 chunk）**——因此无损、不降正确性，同时大幅省 token。
