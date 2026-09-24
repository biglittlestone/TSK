---
description: 在 TSK 已落盘/外置的全文里检索（Think-in-Code：返回精确命中行，不把 bulk 读进上下文）
argument-hint: <查询词...>
---
运行：`tsk search "$1"`，把结果汇报给用户。
- 用途：当需要「精确原文/代码示例/API 签名/日志关键行」时，用它对已分析/索引过的内容按需取回，而不是把大文件读进上下文。
- 检索范围：TSK 本轮已 externalize/落盘的全文（`~/.tsk/<会话>/ext/*.txt`）。
- 返回：按相关性排序的命中（路径 + 精确行窗 snippet，原文保留）。
