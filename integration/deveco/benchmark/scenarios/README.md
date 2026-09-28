# DevEco 10 场景种子（可复现）

每个场景 = 一条 ≥30 有含义轮的 deveco 会话。运行协议与完整设计见 `docs/DEVECO-BENCHMARKS.md` 与本 README。

## 种子结构
```
S{n}-<name>/
  task.md         → 发给 agent 的多阶段任务 prompt（编码 ≥30 有含义轮推进，非随机）
  seed.sh         → 生成确定性 workspace 基线（run 前在本场景空白目录执行）
```
## 跑法（对拍）
1. `bash S{n}-*/seed.sh <workdir>` 在空目录生成基线。
2. on/off 各跑一条 ≥30 轮会话：
   - on  = tsk 插件 + `tsk compress`+KB（插件常驻）
   - off = 移除 ~/.config/deveco/plugin/tsk.ts
3. 测量：会话真实 input token、`tsk report` 输出压缩 saved、`tsk search` 无损回读命中、正确性(构建/测试最终通过)。
4. 判定：on total input < off 且 正确性持平 ⇒ tsk 真省不破。

## 各场景一句话
| # | 会话内核 | 主打收益 |
|---|---|---|
| S1 | ArkTS 笔记 App：实现+每步 hvigor build 排错 | compress |
| S2 | 多文件重构+git 历史操作 | compress + KB |
| S3 | 脚手架+依赖构建+think-in-code 汇总 | compress + 沙箱 |
| S4 | **超长日志/测试失败排查** | compress + B 分节 + BM25 |
| S5 | 多文件研读→设计文档 | 沙箱 + BM25 |
| S6 | UI 迭代+反复构建 | compress |
| S7 | git 200+commit 考古(bisect) | git 大输出 + KB |
| S8 | 性能/内存定位(流式大输出) | compress + 沙箱 + B |
| S9 | 全量 TDD 红绿重构 | compress + KB |
| S10 | 多模块并行构建+集成排错 | compress + B + 套娃守卫 |

> 依赖：git、node、均有 bash 的 opencode 模型。鸿蒙场景(S1/S3/S6/S10)建议有 hvigor/harmonyos 工具链，否则退化为通用构建。