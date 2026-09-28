# TSK 架构（基于实证方向重构）

> 方向结论（本会话实测）：**只有输出压缩系是真实收益**（rtk 式逐命令 + KB/BM25 分节检索 + report 度量）。
> 输入 read 骨架、状态机 policy 等均无收益 → 已删除。架构围绕「收益轴 = 输出压缩 → 检索 → 度量」组织。

## 目标架构（两块集成面 + 共享 Rust 核心）

```
┌──────────── shared Rust core (tsk) ────────────┐
│  search.rs   BM25 索引 + 精确 chunk 检索(C 高亮) │  ← 回读
│  sandbox.rs  exec/analyze 沙箱计算(只回 stdout)   │  ← 溯源
│  report      #4 记 deveco 输出压缩 saved         │  ← 度量
│  inject.rs   session_start/user_prompt/precompact│  ← #2 文风(off)+resume
│  ledger/storage/snapshot/config                  │
└────────────────────────────────────────────────┘
   ▲ spawn tsk            ▲ MCP(tools)          ▲ tsk compress(建议落)
┌──────────────┐   ┌────────────────┐   ┌──────────────────┐
│ deveco 插件   │   │ Claude MCP      │   │ (待建) tsk compress
│ tool.execute  │   │ tsk_analyze/    │   │  逐命令输出压缩引擎进核心
│ .after 输出压缩│   │ search/exec     │   │  以替代插件内联 compactByCommand
└──────────────┘   └────────────────┘   └──────────────────┘
```

## 三根收益支柱（都活着）
1. **输出压缩**：deveco `tool.execute.after` 逐命令语义压缩(rtk 式, ls/git/find/grep/wc/du/df/ps/test…) + 无损落盘 `.tsk/kb` + B 分节。
2. **回读走 BM25**：`tsk search` 精确片段(带 dimension 高亮)；套娃守卫保证读自身 KB 直通。
3. **度量**：`tsk report --all` 报「deveco 输出压缩 saved tok/N 次」。

## 已删死代码（本轮）
- `core/src/compress/{externalize,skeleton}`（唯一写者 pretooluse 已删）
- `core/src/policy.rs`（无调用者）
- config 骨架/外置常量：`SKELETON_HEAD/TAIL/MIN_LINES/ROUTE_THRESHOLD/EXTERNALIZE_THRESHOLD`
- read 骨架 hook(PreToolUse/PostToolUse)+ 16+3 测试 + 8 helper；deveco L2 死分支

## 待办重构（建议，需单独立项）
2. **config `output_compression`**：#2 保留，若最终弃用文风可整体移除该装置。
3. Claude MCP tools 与 `tsk search/analyze/exec` 语义对齐(高亮已入 search)。

## 已完成（本轮）
1. **`tsk compress`（已落地，核心迁移）**：`core/src/compact.rs` = 逐命令输出压缩引擎（ls/tree/find/git status/log/diff/grep/rg/wc/du/df/ps/curl/wget/测试构建），5 个 Rust 单测；CLI `tsk compress`（stdin raw + argv cmd）。deveco 插件 tool.execute.after 改 spawn `tsk compress`（薄桥，`{}`=未命中回退内联）。收益实测 80-96%（git log 80% / diff 96% / ls 94% / node-test 96% / grep 90% / find 89%）。Rust 测试 7+18+15 全绿 + smoke 39/39。

## 保留(不应动)
- 输出压缩(逐命令+B/KB/BM25)、套娃守卫、NO_EMULATOR、compact→resume、MCP 沙箱、#2 文风路径。