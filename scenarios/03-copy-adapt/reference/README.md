# 参考 Todo 应用（copy-adapt 底座）

一个小型、完整的前端 Todo（localStorage 持久化）。被用作「复制到新项目再适配」的参考底座。**目标项目应复制本目录到 `solution/`，然后改造成看板。**

## 结构
- `index.html` — UI 骨架（header/toolbar/list/footer）
- `style.css` — 深色主题样式
- `app.js` — 逻辑，已分成三层：
  - **store 数据层**：`load()/save()/nextId()`，localStorage 纯函数
  - **render 视图层**：`itemEl()/render()/byFilter()/escapeHtml()`
  - **actions 交互层**：`add/toggle/remove/toggleAll/clearDone/move`（数据改动 → `store.save` → `refresh()`）

## UI 契约（改造成看板时需保留或平移的挂载点）
- `#input` 输入框、`#addbtn` 添加按钮、`#list` 列表容器
- 每项：`checkbox 完成` + `span.txt 文本` + `button.del 删除`；`li.draggable` 支持拖拽换序
- `#count` 总数徽章、`#filter` 筛选(all/active/done)、`#left` 剩余计数

## 关键模式（适配看板时可复用）
1. 单一数据数组 + 每次操作「load → 纯函数变换 → save → refresh」。看板可换成状态对象 `{todo:[],doing:[],done:[]}` 三数组，或单数组 + 每项带 `status`。
2. `escapeHtml` 防 XSS——看板卡片文案仍须转义。
3. 事件委托到 `#list`；拖拽通过 `dragstart/dragover/drop` + `dataTransfer` 记录 id。
4. 无框架、无构建、无外链；`node --check` 可校验。

## 现状缺陷（可在看板中顺手修复）
- `move()` 只保证跨行换序，未做跨列。
- 编辑靠「删除重加」，无内联 edit。
- 无持久化的列状态；无拖拽落点高亮。