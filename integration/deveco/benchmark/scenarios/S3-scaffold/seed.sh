#!/usr/bin/env bash
# integration/deveco/benchmark/scenarios/S3-scaffold —— $1 根基线
set -euo pipefail
cd "${1:?workdir}"
printf '%s\n' '# scaffold' > README.md && mkdir -p src/A src/B src/C
echo [seed] integration/deveco/benchmark/scenarios/S3-scaffold ok
