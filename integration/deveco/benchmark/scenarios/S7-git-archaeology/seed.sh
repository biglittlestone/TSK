#!/usr/bin/env bash
# S7-git-archaeology —— 220 commit 仓库基线
set -euo pipefail
cd "${1:?workdir}"
if [ ! -d .git ]; then git init -q; for i in $(seq 1 220); do echo "export const x=1; // ok" >> calc.js; git add .; git commit -qm "c$i"; done; printf '// regression marker\n' >> calc.js; git add .; git commit -qm final; fi
git rev-list --count HEAD | xargs echo "[S7] commits:"
