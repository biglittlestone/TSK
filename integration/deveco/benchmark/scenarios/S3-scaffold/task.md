# 鸿蒙·ArkTS · 脚手架+依赖+think-in-code 汇总

工作目录已用 seed 生成基线（勿删）。按下列**有序步骤**逐条执行；每行=必须完成的一个有含义步骤，**禁止跳步/随机换工具/跳过构建-验证**；每步引用上一步结果。走完即停。

## 有序步骤（30 个有含义轮）
1. 搭工程
2. install(大输出)
3. build首过
4. 确认产物
5. 实现A,build修
6. 补A边界,build修
7. tsk analyze 模块A
8. 实现B,build修
9. 补B边界,build修
10. tsk analyze 模块B
11. 实现C,build修
12. 补C边界,build修
13. tsk analyze C
14. tsk analyze 全仓聚合
15. tsk search 查精确API
16. 写overview第1章
17. 写第2章
18. 写第3章
19. 复查build全绿
20. 收尾
21. 逐项校验 1：跑全量测试查看通过
22. 逐项校验 2：build/编译通过校验
23. 逐项校验 3：git status 看改动是否如预期
24. 逐项校验 4：tsk search 回看任一被压缩原文确认无损
25. 逐项校验 5：对照上一轮结果确认推进
26. 逐项校验 6：产物/目录结构核查
27. 逐项校验 7：跑全量测试查看通过
28. 逐项校验 8：build/编译通过校验
29. 逐项校验 9：git status 看改动是否如预期
30. 逐项校验 10：tsk search 回看任一被压缩原文确认无损

命令输出常很大：构建/测试/日志用命令浓缩（只看错误段，别整读）；需要原文精确行时用 tsk search。完成后 ≤200 字收尾。DO NOT ASK ANY QUESTION.
