# deveco 全量端到端收益（ON 场，deveco run + TSK 实际运行）

口径：deveco run 真实驱动 GLM-5.1 跑 benchmark case；TSK 插件实际压缩（journal 记账 saved）+ 实际回读（日志）；净收益 = saved − 回读×75；完成度 = 构建出 hap。

**69 个 case 跑完 | 完成 ✅ 10 (14%) | 有收益 12 | 总 saved 81051 tok | 总净收益 77076 tok**

| case | 压缩 | saved | 净收益 | 完成度 |
|---|---|---|---|---|
| bootstrap-audio-recorder | 22 | 24181 | 23356 | ❌ |
| bootstrap-skymusic | 10 | 18573 | 17898 | ✅ |
| bootstrap-tax-refund-calc | 26 | 14860 | 14185 | ✅ |
| bootstrap-local-music-player | 8 | 6028 | 5803 | ✅ |
| bootstrap-wuge-groceries | 7 | 4146 | 3621 | ❌ |
| bootstrap-gomoku-15x15 | 5 | 3123 | 2823 | ✅ |
| bootstrap-elder-medication | 4 | 2677 | 2377 | ✅ |
| bootstrap-self-discipline-suite | 4 | 1953 | 1953 | ✅ |
| bootstrap-duoyoubao-mall | 4 | 2136 | 1911 | ✅ |
| bootstrap-ncba-campus-guide | 3 | 1713 | 1638 | ✅ |
| bootstrap-time-capsule | 1 | 833 | 758 | ✅ |
| bootstrap-calculator | 1 | 828 | 753 | ✅ |
| bootstrap-ai-subtitle | 0 | 0 | 0 | ❌ |
| bootstrap-bazi-daily-fortune | 0 | 0 | 0 | ❌ |
| bootstrap-doc-scan-organizer | 0 | 0 | 0 | ❌ |
| bootstrap-emotion-wellness | 0 | 0 | 0 | ❌ |
| bootstrap-fruit-slice | 0 | 0 | 0 | ❌ |
| bootstrap-healthy-life | 0 | 0 | 0 | ❌ |
| bootstrap-hong-paint-editor | 0 | 0 | 0 | ❌ |
| bootstrap-huabao-fund | 0 | 0 | 0 | ❌ |
| bootstrap-id-photo-studio | 0 | 0 | 0 | ❌ |
| bootstrap-legend-life-official | 0 | 0 | 0 | ❌ |
| bootstrap-memory-card-game | 0 | 0 | 0 | ❌ |
| bootstrap-mortar-game | 0 | 0 | 0 | ❌ |
| bootstrap-pomodoro-focus | 0 | 0 | 0 | ❌ |
| bootstrap-voting-system | 0 | 0 | 0 | ❌ |
| cc-ark-001 | 0 | 0 | 0 | ❌ |
| cc-ark-002 | 0 | 0 | 0 | ❌ |
| cc-ark-003 | 0 | 0 | 0 | ❌ |
| cc-ark-004 | 0 | 0 | 0 | ❌ |
| cc-ark-005 | 0 | 0 | 0 | ❌ |
| cc-ark-006 | 0 | 0 | 0 | ❌ |
| cc-ark-007 | 0 | 0 | 0 | ❌ |
| cc-ark-008 | 0 | 0 | 0 | ❌ |
| cc-ark-009 | 0 | 0 | 0 | ❌ |
| cc-ark-010 | 0 | 0 | 0 | ❌ |
| cc-ark-011 | 0 | 0 | 0 | ❌ |
| cc-ark-012 | 0 | 0 | 0 | ❌ |
| cc-ark-013 | 0 | 0 | 0 | ❌ |
| cc-ark-014 | 0 | 0 | 0 | ❌ |
| cc-ark-015 | 0 | 0 | 0 | ❌ |
| cc-ark-016 | 0 | 0 | 0 | ❌ |
| cc-ark-017 | 0 | 0 | 0 | ❌ |
| cc-ark-018 | 0 | 0 | 0 | ❌ |
| cc-ark-019 | 0 | 0 | 0 | ❌ |
| cc-ark-020 | 0 | 0 | 0 | ❌ |
| cc-ark-021 | 0 | 0 | 0 | ❌ |
| cc-ark-022 | 0 | 0 | 0 | ❌ |
| cc-ark-023 | 0 | 0 | 0 | ❌ |
| cc-ark-024 | 0 | 0 | 0 | ❌ |
| cc-ark-025 | 0 | 0 | 0 | ❌ |
| cc-ark-026 | 0 | 0 | 0 | ❌ |
| cc-ark-027 | 0 | 0 | 0 | ❌ |
| cc-ark-028 | 0 | 0 | 0 | ❌ |
| cc-ark-029 | 0 | 0 | 0 | ❌ |
| cc-ark-030 | 0 | 0 | 0 | ❌ |
| cc-ark-031 | 0 | 0 | 0 | ❌ |
| cc-ark-032 | 0 | 0 | 0 | ❌ |
| cc-ark-033 | 0 | 0 | 0 | ❌ |
| cc-ark-034 | 0 | 0 | 0 | ❌ |
| cc-ark-035 | 0 | 0 | 0 | ❌ |
| cc-ark-036 | 0 | 0 | 0 | ❌ |
| cc-ark-038 | 0 | 0 | 0 | ❌ |
| cc-ark-039 | 0 | 0 | 0 | ❌ |
| cc-ark-040 | 0 | 0 | 0 | ❌ |
| cc-ark-044 | 0 | 0 | 0 | ❌ |
| cc-ark-045 | 0 | 0 | 0 | ❌ |
| cc-ark-046 | 0 | 0 | 0 | ❌ |
| demo-photo-picker | 0 | 0 | 0 | ❌ |

**观察**：TSK 收益集中在模型实际触发大 bash 输出的 case（audio-recorder 23K / skymusic 18K / tax-refund 14K）；复杂 case（cc-ark 等）多数 0 收益且未构建出 hap（模型用 read 直通 + 未完成构建）——端到端实际收益由模型行为决定，显著低于重放上限。
