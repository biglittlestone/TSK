#!/usr/bin/env bash
# 生成案例工作区：template/ -> .work/seed/ + 一个只有规格与数据的空 git 仓库。
# 实现与优化全部由 deveco 在循环里完成，这里不给任何成品代码。
# 用法：bash gen-seed.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SEED="${TSK_CASE_WORK:-$HERE/.work}/seed"

rm -rf "$SEED"
mkdir -p "$SEED"
cp -r "$HERE/template/." "$SEED/"

cd "$SEED"
git init -q
git config user.name "tsk-case"
git config user.email "tsk-case@example.com"
git config commit.gpgsign false

GIT_AUTHOR_DATE="2026-09-01T09:00:00+08:00" GIT_COMMITTER_DATE="2026-09-01T09:00:00+08:00" \
  git add -A
GIT_AUTHOR_DATE="2026-09-01T09:00:00+08:00" GIT_COMMITTER_DATE="2026-09-01T09:00:00+08:00" \
  git commit -q -m "chore: 图书管理系统规格与书目数据（待实现）"

chmod +x verify.sh 2>/dev/null || true

echo "seed 生成于 $SEED（空白起点：SPEC.md + reference/books.json）"
git --no-pager log --oneline
