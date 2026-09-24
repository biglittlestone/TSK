参考目录 `reference/` 是一个可用的 **Todo 应用**（index.html + app.js，localStorage 持久化、显示一列待办、可增删勾选）。

任务：**把 `reference/` 复制到 `solution/` 作为底座，并改造成「看板（Kanban）」**：
- 三列：待办 / 进行中 / 已完成，每列有标题与计数。
- 列表项可**右键或长按移动**到相邻列（或提供 向左/向右 按钮）。
- 保留 localStorage 持久化；新增「新增卡片」在每列顶部；可编辑卡片文案、可删除。
- 保持代码结构清晰（可拆成 render/action/data 三部分），不用框架。

验证脚本 `verify.mjs`：
1. 新增卡片进入「待办」列；
2. 移动卡片到「进行中」后，状态/所在列变化；
3. 完成后在「已完成」列；删除生效；
4. 刷新后数据仍在（localStorage）。

对比 reference 与 solution，说明你复用了哪些模式、改了什么。
