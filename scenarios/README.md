# TSK 端到端基准（真实 Claude Code 多轮，含回读收益口径）

目的：用**真实模型 + 真实多轮对话**量化 TSK 收益，且**收益计入「回读导致的 token」**（不只看乐观 saved）。

## 3 个场景（都 ≤30 轮对话，真调模型，非 hap）

| # | 场景 | 参考来源 | 强制 ≥30 轮的关键 | 成功判据 |
|---|---|---|---|---|
| 01 | **网页计算器** `01-calc-web/` | deveco `task-004-calculator`（转 web，非 hap） | 单页 app：spec 读取→布局→JS 计算逻辑→多用例无头验证→修错→回报 | `solution/index.html` + 四则/除零/连续运算全过 |
| 02 | **图书管理系统 CRUD** `02-library-crud/` | deveco `task-021-audio-recorder` 风格（转 CRUD web） | 数据模型→三面板 UI→增删改查→借还/逾期→搜索筛选→localStorage 持久化→无头端到端验证 | US1/2/3 各验证过 + 刷新持久化 |
| 03 | **复制参考并适配成看板** `03-copy-adapt/` | 参考一个 Todo app，复制后改造 | 大/多文件参考（app.js/index.html/README）→复制→改三列看板→拖动/移动→localStorage→对比 reference 说明复用 | solution kanban + 移动/完成后验证 |

每个 seed 含：`task.md`（agent go prompt，同 deveco Phase-4：不提问、直接干、跑验证）+ `reference-spec.md`/`reference/`（大文件，故意让 TSK 骨架/检索）`+ success.sh` + 空 `solution/`。

> 为什么 ≥30 轮自然发生：多模块（UI+逻辑+持久化）+ 一个无头验证脚本的编写与多次修错 + 逐用例跑并改 + 产物报告。每步含多次 read（spec/项目）/bash（node 验证）/edit，天然过 30。

## 测量协议（on/off 各跑一遍，真模型）

```
# OFF（TSK 关，fs 无损）
claude -p "$(cat task.md)" --output-format json --dangerously-skip-permissions   # 在场景 workspace 跑，禁全局 TSK
# ON（TSK 开）
claude -p "$(cat task.md)" --output-format json --dangerously-skip-permissions   # TSK hooks 生效
```
- **token 统计**：读该会话的 `--output-format json`（含每 step 的 tokens.input/cache）或 proxy jsonl，累加**每轮真实 input token**（含 cache）。
- **回读口径**：TSK 的 `tsk report` 直接给 `saved`（乐观）与**悲观上限**（若把被压原文全部回读 → 负净省）；并记 `retrievals` 次数。净收益 = saved − 注入 − 实际回读字节。
- **对比**：on/off 的 `pass`（success.sh）持平 + on total input token < off ⇒ TSK 真省且不破 case。

## 与 deveco 用例的关系
从 deveco bootstrap case 借「单页 app 从 0 实现」的形态，但**不编译 hap**——改 web/Node 工具（计算器/图书/看板），可在任何有 Node + 浏览器/无头工具的环境跑，无需 HarmonyOS 工具链。
