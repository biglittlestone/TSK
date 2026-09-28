参考目录 `reference/` 是一个可用的 **Todo 应用**（index.html + style.css + app.js，localStorage 持久化、含筛选/全选/清除已完成/拖拽换序，逻辑已分 store/render/actions 三层）。

任务：**把 `reference/` 整个复制到 `solution/` 作为底座，再改造成「看板（Kanban）」**：
- 三列：待办 / 进行中 / 已完成，每列有标题与计数。
- 列表项可**右键或长按移动**到相邻列（或提供 向左/向右 按钮）。
- 保留 localStorage 持久化；新增「新增卡片」在每列顶部；可编辑卡片文案、可删除。
- 复用 reference 的分层结构（store/render/actions），不用框架；`style.css` 沿用并扩展出三列布局。

验证脚本 `verify.mjs`（无头，优先系统 Google Chrome）：
1. 新增卡片进入「待办」列；
2. 移动卡片到「进行中」后，状态/所在列变化；
3. 完成后在「已完成」列；删除生效；
4. 刷新后数据仍在（localStorage）。

对比 reference 与 solution，说明你复用了哪些模式、改了什么。
