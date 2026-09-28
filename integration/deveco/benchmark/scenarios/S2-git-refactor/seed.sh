#!/usr/bin/env bash
# S2-git-refactor —— 6 文件 + 6 commit 基线
set -euo pipefail
cd "${1:?workdir}"
if [ ! -d .git ]; then git init -q; for i in 1 2 3 4 5 6; do printf 'export const f%s = %s;\n' "$i" "$i" > mod$i.js; git add .; git commit -qm "feat: mod $i"; done; fi
git rev-list --count HEAD | xargs echo "[S2] commits:"
