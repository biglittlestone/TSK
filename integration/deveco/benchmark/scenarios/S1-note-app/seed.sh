#!/usr/bin/env bash
# integration/deveco/benchmark/scenarios/S1-note-app —— $1 根基线
set -euo pipefail
cd "${1:?workdir}"
printf '%s\n' '# 笔记App参考(三页+localStorage)' > reference-spec.md
echo [seed] integration/deveco/benchmark/scenarios/S1-note-app ok
