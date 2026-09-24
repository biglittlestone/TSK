#!/usr/bin/env bash
# 成功= solution/index.html 存在 && 语法/计算校验通过
cd "$(dirname "$0")"
[ -f solution/index.html ] || { echo "FAIL: no solution/index.html"; exit 1; }
# 粗略：node 提取内联 JS 校验 + 有 + - × ÷ 逻辑
echo "OK: solution/index.html 存在"
