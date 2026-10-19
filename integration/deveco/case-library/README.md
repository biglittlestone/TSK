# 真实使用案例：图书管理系统「从 0 实现 + 常驻优化」（deveco）

**目的**：在一条**真实、持续**的 deveco 工作流里看 TSK 的收益——从 0 实现一个前后端分离的图书管理系统，然后一轮轮优化它（界面 / 后端 / 重构），全程 git 管理。TSK 的收益就藏在这些轮次天然产生的大工具输出里：`git status/diff/log`、hvigor 构建日志、`bun test` 输出、读多文件源码、grep/find 大目录。

> 起点是**空白**：seed 里只有规格（`SPEC.md`）和书目数据（`reference/books.json`）。**实现和优化全部由 deveco 完成**，不由人手写。

## 硬约束

- **只构建，不运行**：前端是 HarmonyOS 应用，模拟器/设备/真机**永久禁用**。前端只做 `hvigorw assembleHap` 构建验证（产 hap）。
- **GLM-5.1 免费额度 50M tok/日** 是硬约束。

## 场景覆盖（backlog 55 轮）

前 8 轮是"开发新功能"，但真实 agent 工作远不止于此。按业界 agentic benchmark 的分类补全：

| 类 | 轮次 | 场景 | 对齐 benchmark |
|---|---|---|---|
| 开发 | 1–8 | 从 0 实现 → 增量功能/性能/重构/UI | devEco `project_bootstrap`；Commit0 |
| **缺陷** | 9–13 | issue 驱动修复 · 缺陷定位 · **回归排查(git log/bisect)** · 边界逻辑 · 转义健壮性 | SWE-bench（issue→patch）；SWE-smith/BugsInPy/Defects4J（缺陷定位） |
| **质量** | 14–18 | 性能优化 · **UI 增量** · 重构 · 测试补充 · **代码审查** | devEco `ui_increment`；CR-bench/CodeReviewer |
| **工程** | 19–21 | **安全加固** · 依赖升级/迁移 · 文档 | RepoAudit/CWE-bench；Terminal-Bench |
| **理解** | 22 | 代码考古/onboarding | （研读类） |
| **全库审查** | 23 | **全库代码审查**（审 server.js 全部 567 行 + app 全部 5 页，非仅最近改动） | CodeReviewer/CR-bench 全库模式 |
| **大重构** | 24–26 | **单文件拆多模块**（567 行 → db/validators/repository/routes）· **前端组件层抽取** · **server+app 共用类型常量**（新建 shared/） | Commit0/SWE-gym 跨文件重构 |
| **数据质量** | 27 | **全量审计 294KB books.json**（ISBN 连字符/三位年 dueDate 统计+样本）+ 实现 issue#2 ISBN 归一化 | DABench/数据治理；issue→patch |
| **测试** | 28 | 端到端集成测试套件（借→归还→并发→删书全链） | SWE-bench test 完备性 |
| **安全** | 29 | **grep 全库输入路径与危险模式**（拼 SQL/未校验落库/静默吞错）并修 | CWE-bench/RepoAudit 扫描模式 |
| **理解** | 30–31 | 全模块架构文档（ER+状态机+前端数据流）· **git 全史考古**（27+ 提交追踪借阅逻辑演变） | onboarding；代码考古 |
| **高收益轮（配方 A）** | 32–47 | **强制工具整读** books.json 各角度（审计/统计/迁移/画像/时间/字典）· git 考古（前端/测试/schema/API/issue/构建） | 段内 e2e 3.66% |
| **新数据源** | 48–55 | 引入 1.4MB `loans_history.json`（9000 条借阅历史），整读审计/时间/读者/借期/对照/联合/热门/字典 | 段内 e2e 18.5% |

> `ISSUES.md` 是 issue 素材（8 条报上来的问题，**是报告不是规格**——需自行复现定位）。轮 11 的回归排查依赖前 10 轮积累的 git 历史；轮 31 的全史考古依赖**全部** 31 轮前的历史，因此放在最后。

### 为什么 23–31 轮是"高收益"设计

TSK 的 saved 只来自**被压缩的大工具输出**（输出 >1600 字符 + 命令被识别，或 hvigor/bun test 构建标记）。2–7 轮收益为 0、小改轮恒 ~437，就是因为那些轮次几乎没有大输出。**23–31 轮每轮都按机制反向设计成"必然产生大输出"**：

| 轮 | 主打杠杆 | 对照历史量级 |
|---|---|---|
| 23 | 多文件整读 + `git diff` | 轮 18 审查 26,452 |
| 24–26 | 跨文件**大 diff** + 反复跑测试/构建 | 轮 16 重构 11,864 · 轮 8 构建 19,521 |
| 27 | **大文件整读/grep 大输出**（294KB，头号杠杆） | 读一遍 ≈ 70k tok 量级 |
| 28 | `bun test` 大输出 | 测试类 |
| 29 | `grep -rn` 全库大输出 | grep 类 |
| 30–31 | 多文件整读 / **纯 diff 挖掘** | 轮 22 考古 5,372 · 轮 11 回归 8,459 |

排序上 git 历史只增不减，diff/考古轮越靠后 diff 越大 → 考古类（31）压轴。

### 32–55 轮：配方 A 与新数据源（把 e2e 从 1.6% 推到 3.9%）

**配方 A（最强杠杆）**：让模型**用 cat/read 工具整读全量文件**，而非写脚本 `readFileSync` 只打印统计。轮 27（写脚本）saved 仅 10,418，轮 32（加一句「禁止脚本、必须用工具逐段读完」）saved 104,156——**唯一差别就是那句 prompt**。原理：工具输出被压成摘要，而读大文件的 input 成本≈0（只进 ~200 token），saved 是纯赚，故单轮 e2e 可达 20–49%。

**新数据源**：`reference/loans_history.json`（`tools/gen-loans-history.mjs` 确定性生成，9000 条、1.4MB，埋 5 类数据问题供审计轮挖）。收益与"被读文件字节数"近似线性：books.json 294KB → 单轮 73–120k；1.4MB → 单轮 160–563k。

> **边界**：1.4MB 已是真实借阅历史的合理规模，继续冲高只剩"堆更大数据"、会偏离真实案例。55 轮 / 3.92% 是"保持数据真实"前提下的自然上限。

> `ISSUES.md` 是 issue 素材（8 条报上来的问题，**是报告不是规格**——需自行复现定位）。
> 轮 11 的回归排查依赖前 10 轮积累的 git 历史。

## 目录

```
gen-seed.sh        生成空白工作区 .work/seed（SPEC.md + ISSUES.md + reference/books.json + 空 git 仓库）
template/          起点模板
backlog.json       任务清单（55 轮：开发/缺陷/质量/工程/理解 + 23-31 高收益轮 + 32-47 配方A + 48-55 新数据源）
tools/             gen-loans-history.mjs（生成 reference/loans_history.json）
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
