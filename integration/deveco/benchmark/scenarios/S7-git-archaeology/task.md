# 通用·git · 220commit定位回归(bisect)

工作目录已用 seed 生成基线（勿删）。按下列**有序步骤**逐条执行；每行=必须完成的一个有含义步骤，**禁止跳步/随机换工具/跳过构建-验证**；每步引用上一步结果。走完即停。

## 有序步骤（30 个有含义轮）
1. git log --oneline(大)
2. git log --stat
3. git show c50
4. git show c100
5. git show c150
6. git show c220
7. 定区间
8. 区间内逐个git show(),每次看diff判断引入（第1批/次）
9. 区间内逐个git show(),每次看diff判断引入（第2批/次）
10. 区间内逐个git show(),每次看diff判断引入（第3批/次）
11. 区间内逐个git show(),每次看diff判断引入（第4批/次）
12. 区间内逐个git show(),每次看diff判断引入（第5批/次）
13. 区间内逐个git show(),每次看diff判断引入（第6批/次）
14. 区间内逐个git show(),每次看diff判断引入（第7批/次）
15. 区间内逐个git show(),每次看diff判断引入（第8批/次）
16. bisect start主中点
17. git diff看状态
18. 再折半
19. 再折半
20. 再折半
21. 收敛到首个坏commit
22. git log确认坏commit
23. git diff找引入行
24. 写结论
25. 逐项校验 1：跑全量测试查看通过
26. 逐项校验 2：build/编译通过校验
27. 逐项校验 3：git status 看改动是否如预期
28. 逐项校验 4：tsk search 回看任一被压缩原文确认无损
29. 逐项校验 5：对照上一轮结果确认推进
30. 逐项校验 6：产物/目录结构核查

命令输出常很大：构建/测试/日志用命令浓缩（只看错误段，别整读）；需要原文精确行时用 tsk search。完成后 ≤200 字收尾。DO NOT ASK ANY QUESTION.
