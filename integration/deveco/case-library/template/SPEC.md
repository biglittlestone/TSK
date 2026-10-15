# 图书管理系统 —— 实现规格

在**当前目录**从零实现一个前后端分离的图书管理系统。本文件是唯一规格来源。

## 硬约束

- **只构建，不运行**：前端是 HarmonyOS 应用，**永远不要启动模拟器/设备/真机**（`emulator` / `hdc install` / `--device` 等一律禁止）。前端只需能构建出 hap。
- 不要提问，直接做。

## 仓库布局（验收依赖此布局，必须一致）

```
server/      Node/Bun REST API（SQLite 持久化）
  test/      用 bun test 跑的用例
app/         HarmonyOS ArkUI 应用（ArkTS）
reference/   books.json —— 数据权威，不要修改
```

## 后端 `server/`

- 运行时：**Bun**（用 `bun:sqlite` 做持久化，`Bun.serve` 起服务）。
- `package.json` 里提供 `"test": "bun test"`。
- 表：`books` / `members` / `loans`（字段自定，但需覆盖下面的 API 语义）。

### REST 端点

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/books` | 查询参数 `q`（在 title/author/isbn/category 上做包含匹配）、`category`、`status`（`in`/`borrowed`）、`page`（默认 1）、`size`（默认 20）、`sort`（默认 id）。返回 `{total, page, size, items}` |
| GET | `/books/:id` | 单本；不存在返回 404 |
| POST | `/books` | 建书；`title`/`author`/`isbn` 必填，缺则 400 |
| PUT | `/books/:id` | 改 title/author/category；不存在返回 404 |
| DELETE | `/books/:id` | 删书 |
| GET | `/loans` | 借阅记录列表，返回 `{total, items}`（与 `/books` 同信封）；每条含 `active`（未归还）与 `overdue`（未归还且已过应还日） |
| POST | `/loans` | 借出：body `{bookId, member}`；写入借阅记录并把书置为 `borrowed`，`due_date` = 借出日 + **14 天**；书不存在返回 404 |
| POST | `/loans/:id/return` | 归还：记录归还时间，书恢复 `in`；重复归还返回 409 |

- 日期用 `YYYY-MM-DD`。
- 初始书目从 `reference/books.json` 的 `entries` 灌入（1100 本）。`server.js` 直接运行时若库为空则灌种。

### 后端验收

```bash
cd server && bun test      # 必须全绿
```
用例至少覆盖：列表/搜索/过滤/分页、增删改查、借出、归还、重复归还 409、逾期标记。

## 前端 `app/`

- **HarmonyOS ArkUI（ArkTS）**工程，`entry` 模块，包名 `com.example.libraryapp`，targetSdkVersion `26.0.0`。
- 需要 `ohos.permission.INTERNET`。
- 通过 HTTP 调后端（`@kit.NetworkKit`），后端地址写常量即可（本案例不实际运行）。

### 页面

| 页面 | 内容 |
|---|---|
| 首页 | 两个入口：书目浏览、借阅记录 |
| 书目列表 | 列表展示 + 关键词搜索 + 分页；显示书名/作者/分类/在库状态 |
| 书目详情 | 单本详情 + 借出（填借书人）+ 删除 |
| 借阅记录 | 借阅列表 + 归还操作；逾期高亮 |

### 前端验收

```bash
cd app && hvigorw assembleHap
```
产出 `app/entry/build/default/outputs/default/*.hap` 即通过。

## 数据权威

`reference/books.json`：`{generated, total, categories[], entries[]}`，每条 `{id, title, author, isbn, category, publishedYear, status, borrower?, dueDate?, addedAt}`。字段名、ISBN 格式、分类集合都以它为准；**不要读完全部 1100 条**，抽样确认格式即可。

## 完成标准

`server` 测试全绿 **且** `app` 能构建出 hap。
