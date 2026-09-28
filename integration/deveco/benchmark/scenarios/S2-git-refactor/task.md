# 通用·Node · 6文件模块重构+git 逐步提交

工作目录已用 seed 生成基线（勿删）。按下列**有序步骤**逐条执行；每行=必须完成的一个有含义步骤，**禁止跳步/随机换工具/跳过构建-验证**；每步引用上一步结果。走完即停。

## 有序步骤（30 个有含义轮）
1. ls -laR看结构
2. git log --stat历史
3. cat mod1-3
4. cat mod4-6
5. git status干净
6. 定目标架构
7. A→a1,a2改
8. check+a1
9. commit a1
10. check+a2
11. commit a2
12. B→b1,b2改
13. check+b1
14. commit b1
15. check+b2
16. commit b2
17. C→c1,c2改
18. check+c1
19. commit c1
20. check+c2
21. commit c2
22. 改a1→check→commit
23. 改a2→check→commit
24. 改b1→check→commit
25. 改b2→check→commit
26. 跑全测试(大输出)
27. git log --oneline复查
28. git diff首vs末
29. 收尾
30. 逐项校验 1：跑全量测试查看通过

命令输出常很大：构建/测试/日志用命令浓缩（只看错误段，别整读）；需要原文精确行时用 tsk search。完成后 ≤200 字收尾。DO NOT ASK ANY QUESTION.
