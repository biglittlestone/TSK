# 图书管理系统 — 功能规格

**目标**：一个单页图书管理 Web 应用（HTML+CSS+JS，无框架），用 `localStorage` 持久化。

## 角色与页面
- 单一页面，三个面板：**图书列表**、**借还登记**、**搜索/筛选**。

## 数据模型
- 图书 Book：{ id, title, author, isbn, category, status: in|borrowed, borrower?, dueDate?, addedAt }。
- 操作：新增、编辑、删除、借出、归还、按 标题/作者/分类/ISBN 搜索。

## 用户故事
### US1 增删查改（P0）
- 可从表单新增图书（标题、作者、分类、ISBN）；列表显示全部；可删除；可编辑标题/作者。
### US2 借还（P1）
- 在库(in)的书可借出：记录借书人、应还日期（默认 +14 天），状态改 borrowed；可归还。
- 逾期(< today)、即将到期(<=3天)在列表高亮。
### US3 搜索与筛选（P2）
- 顶部搜索框按关键词同时匹配 标题/作者/ISBN/分类；可按分类或借出状态筛选。

## 约束
- 纯前端，单 index.html 或 index.html + app.js + style.css，无后端子，数据存 localStorage。
- 校验：新增时标题、作者、ISBN 非空。
- 用 Node 无头验证：新增→列表出现；借出→状态/借书人生效；归还→回 in；删除→消失；刷新后数据仍在（localStorage）。

## 验收
- 上述 US1/US2/US3 各有一个端到端验证脚本用例并通过。
