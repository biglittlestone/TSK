# deveco 评测集端到端 token 节省统计（当前 TSK，ON 场，无 AB）

评测集：26 个 GLM-5.1 真实会话（sdd-bootstrap，E:\00work\参考\artifact_sdd-bootstrap-0730-1）· 压缩逻辑=当前 tsk.plugin.ts（Rust tsk compress；通用蒸馏已移除，未识别命令放行原文）
口径：会话总 input = 各 task 真实 info.tokens.input；bash 输出原始 tok = Σ⌈工具输出/4⌉；压缩省 tok = Σ(⌈原文/4⌉−⌈压缩后/4⌉，含 TSK 包裹头)；端到端节省率 = 压缩省 tok / 会话总 input。

## 逐任务

| 任务 | 会话总input tok | bash输出原始tok | 压缩省tok | 端到端节省率 |
|---|---|---|---|---|
| task-001-bazi-daily-fortune | 3615001 | 9519 | 8370 | 0.232% |
| task-002-ai-subtitle | 2383598 | 8835 | 4495 | 0.189% |
| task-003-fruit-slice | 1391785 | 145 | 0 | 0.000% |
| task-004-calculator | 1043297 | 144 | 0 | 0.000% |
| task-005-self-discipline-assistant | 5939958 | 2438 | 1436 | 0.024% |
| task-006-ncba-campus-guide | 2793191 | 235 | 0 | 0.000% |
| task-007-doc-scan-organizer | 5664827 | 8891 | 4173 | 0.074% |
| task-008-emotion-wellness | 7897944 | 13463 | 8709 | 0.110% |
| task-009-time-capsule | 2109452 | 146 | 0 | 0.000% |
| task-010-focus-pomodoro | 2832692 | 760 | 0 | 0.000% |
| task-011-voting-system | 2308766 | 148 | 0 | 0.000% |
| task-012-healthy-life | 4174996 | 438 | 0 | 0.000% |
| task-013-local-music-player | 1763644 | 158 | 0 | 0.000% |
| task-014-gomoku-15x15 | 1458311 | 7093 | 5110 | 0.350% |
| task-015-memory-card-game | 1389107 | 151 | 0 | 0.000% |
| task-016-elder-medication | 3235453 | 9459 | 7802 | 0.241% |
| task-017-hong-paint-editor | 6052242 | 306 | 0 | 0.000% |
| task-018-mortar-game | 765429 | 143 | 0 | 0.000% |
| task-019-skymusic-piano | 1640991 | 8081 | 6075 | 0.370% |
| task-020-id-photo-studio | 2645062 | 2277 | 0 | 0.000% |
| task-021-audio-recorder | 1882122 | 4011 | 2148 | 0.114% |
| task-022-huabao-fund | 1223077 | 142 | 0 | 0.000% |
| task-023-tax-refund-calculator | 1457832 | 116 | 0 | 0.000% |
| task-024-duoyoubao-mall | 893227 | 145 | 0 | 0.000% |
| task-025-wuge-grocery | 167576 | 0 | 0 | 0.000% |
| task-026-legend-life-official | 1875145 | 176 | 0 | 0.000% |

## 端到端汇总

```

--- 抽查：task-001-bazi-daily-fortune / devecocli docs read "API参考/ArkUI_方舟UI框架/ArkTS组件/按钮与选择/TextPicker/ts-ba (原文 9001 tok → 压缩 631 tok) ---
devecocli docs (56 标题, 32 接口, 57 参数)：
#### 子组件
#### 接口
TextPicker(options?: TextPickerOptions)
options: TextPickerOptions | 否
#### TextPickerOptions对象说明
range: string[] | string[][]10+ | Resource | TextPickerRangeContent[]10+ | TextCascadeP…
selected: number | number[]10+ | 否 | 是
value: ResourceStr | ResourceStr[] | 否 

```

- 26 个 task 会话总 input：**68,604,725 tok**；其中 bash 输出原始共 77420 tok（占 input 的 0.113%）。
- 触发压缩 13 次（>1600B 大输出 31 次）；压缩省 **48,318 tok**（已含 TSK 包裹头固定开销）。
- **端到端 token 节省：48,318 tok，占全部会话 input 的 0.070%**。
- 说明：TSK 引入的回读（tsk search / cat KB）属额外消耗，本表未计入——无 AB 无法实测真实回读行为；压缩省 tok 已扣除 TSK 自己的固定开销（包裹头/章节地图）。

**压缩器状态：** `devecocli docs read`（评测集 bash 主力）已落地 rtk 式语义压缩——保留文档标题/API 签名/参数名:类型，丢正文/说明列/代码块，0 回读。
通用蒸馏已移除：其余未识别命令放行原文（不做丢信息压缩）。剩余 31−13=18 个未触发的大输出是放行的非 devecocli 命令（node/python 等）。
