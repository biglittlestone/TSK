#!/usr/bin/env bash
# integration/deveco/benchmark/scenarios/S6-ui-iterate —— $1 根基线
set -euo pipefail
cd "${1:?workdir}"
printf '%s\n' '# ui 3页' > index.uidef
echo [seed] integration/deveco/benchmark/scenarios/S6-ui-iterate ok
