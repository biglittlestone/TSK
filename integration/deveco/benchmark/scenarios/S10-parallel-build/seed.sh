#!/usr/bin/env bash
# integration/deveco/benchmark/scenarios/S10-parallel-build —— $1 根基线
set -euo pipefail
cd "${1:?workdir}"
for m in A B C; do mkdir -p m$m; printf 'export const mod%s;\n' "$m" > m$m/mod.js; done
echo [seed] integration/deveco/benchmark/scenarios/S10-parallel-build ok
