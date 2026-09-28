#!/usr/bin/env bash
# 成功 = solution/index.html 存在 && verify.mjs 存在且为合法 JS（真实验证由模型在会话内跑）
cd "$(dirname "$0")"
[ -f solution/index.html ] || { echo "FAIL: no solution/index.html"; exit 1; }
[ -f solution/verify.mjs ] || { echo "FAIL: no solution/verify.mjs"; exit 1; }
if command -v node >/dev/null 2>&1; then node --check solution/verify.mjs || { echo "FAIL: verify.mjs invalid JS"; exit 1; }; fi
echo "OK: solution/index.html + verify.mjs 存在"