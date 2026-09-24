请基于 `reference-spec.md` 实现图书管理 Web 应用，产出在 `solution/`（index.html 或 index.html+app.js+style.css，自包含无框架）。不要提问。完成后写一个 Node 无头验证脚本 `verify.mjs`，覆盖并修复到全过：

1. 新增一本《三体·黑暗森林》→ 列表出现。
2. 借出 → 状态 borrowed、借书人/应还日期生效；归还 → 状态回 in。
3. 删除 → 列表消失。
4. 刷新（重载页面数据源）→ 数据仍在（localStorage 持久化）。
5. 搜索 "三体" 命中，按分类筛选生效。

说明实现与每个验证结果。
