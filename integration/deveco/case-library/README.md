# 真实使用案例：图书管理系统「从 0 实现 + 常驻优化」（deveco）

**目的**：在一条**真实、持续**的 deveco 工作流里看 TSK 的收益——从 0 实现一个前后端分离的图书管理系统，然后一轮轮优化它（界面 / 后端 / 重构），全程 git 管理。TSK 的收益就藏在这些轮次天然产生的大工具输出里：`git status/diff/log`、hvigor 构建日志、`bun test` 输出、读多文件源码、grep/find 大目录。

> 起点是**空白**：seed 里只有规格（`SPEC.md`）和书目数据（`reference/books.json`）。**实现和优化全部由 deveco 完成**，不由人手写。

## 硬约束

- **只构建，不运行**：前端是 HarmonyOS 应用，模拟器/设备/真机**永久禁用**。前端只做 `hvigorw assembleHap` 构建验证（产 hap）。
- **GLM-5.1 免费额度 50M tok/日** 是硬约束。

## 场景覆盖（backlog 22 轮）

前 8 轮是"开发新功能"，但真实 agent 工作远不止于此。按业界 agentic benchmark 的分类补全：

| 类 | 轮次 | 场景 | 对齐 benchmark |
|---|---|---|---|
| 开发 | 1–8 | 从 0 实现 → 增量功能/性能/重构/UI | devEco `project_bootstrap`；Commit0 |
| **缺陷** | 9–13 | issue 驱动修复 · 缺陷定位 · **回归排查(git log/bisect)** · 边界逻辑 · 转义健壮性 | SWE-bench（issue→patch）；SWE-smith/BugsInPy/Defects4J（缺陷定位） |
| **质量** | 14–18 | 性能优化 · **UI 增量** · 重构 · 测试补充 · **代码审查** | devEco `ui_increment`；CR-bench/CodeReviewer |
| **工程** | 19–21 | **安全加固** · 依赖升级/迁移 · 文档 | RepoAudit/CWE-bench；Terminal-Bench |
| **理解** | 22 | 代码考古/onboarding | （研读类，read-CCR 主场） |

> `ISSUES.md` 是 issue 素材（8 条报上来的问题，**是报告不是规格**——需自行复现定位）。
> 轮 11 的回归排查依赖前 10 轮积累的 git 历史。

## 目录

```
gen-seed.sh        生成空白工作区 .work/seed（SPEC.md + ISSUES.md + reference/books.json + 空 git 仓库）
template/          起点模板
backlog.json       任务清单（22 轮：开发/缺陷/质量/工程/理解）
run-persistent.sh  常驻循环驱动（每轮独立 deveco 会话 → 验收 → git 提交 → 归因）
ab-read-ccr.sh     read-CCR 完成度影响性 A/B
acceptance/run.mjs 独立验收套件（agent 无关，按 SPEC 契约打 HTTP）
collect.mjs        逐轮 + 累计收益收集器
.work/             [生成物] 工作区 seed/ 与运行产物 runs/
```

## 跑法

```bash
bash gen-seed.sh                 # 生成空白工作区
bash run-persistent.sh --count 8 # 跑前 8 轮（每轮一条 backlog）
bash run-persistent.sh --only 1  # 只跑第 1 轮（从 0 实现）
```

跑完在 `docs/DEVECO-CASE-LIBRARY-REPORT.md` 出收益报告。

## 度量口径

- **TSK 省**：`Σ saved_tokens`（`<seed>/.tsk/injections.jsonl` 的 `sandbox-output` 条目），逐轮取 journal 行区间增量。
- **回读成本**：该轮会话里 `tsk search` / 读 KB 的次数 × 75 tok（沿用实测中值）。
- **净收益** = 省 − 回读。
- **端到端%** = 该轮省 / 该轮会话 input tokens（`deveco export <session>`）。
- **完成度** = `verify.sh`（`server/` 存在则 `bun test` 全绿；`app/` 存在则构建出 hap）。

## 前置

- `tsk` 在 PATH；TSK deveco 插件已装（`%USERPROFILE%\.deveco\plugin\tsk.ts`）。
- DevEco Studio（默认 `D:/DevEco Studio`，可用 `DEVECO_STUDIO` 覆盖）。
- 模型：`deveco/GLM-5.1`（可用 `--model` / `TSK_CASE_MODEL` 覆盖）。
