#!/usr/bin/env bash
# integration/deveco/benchmark/scenarios/S5-kb-build —— $1 根基线
set -euo pipefail
cd "${1:?workdir}"
for i in 1 2 3 4 5; do printf 'export function api%s(p){return p+%s;}\n' "$i" "$i" > f$i.js; done
echo [seed] integration/deveco/benchmark/scenarios/S5-kb-build ok
