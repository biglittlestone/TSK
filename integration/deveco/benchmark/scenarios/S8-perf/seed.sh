#!/usr/bin/env bash
# integration/deveco/benchmark/scenarios/S8-perf —— $1 根基线
set -euo pipefail
cd "${1:?workdir}"
printf '%s\n' 'const b=[];for(let i=0;i<200000;i++)b.push(i*i%7);console.log(b.length)' > workload.js
echo [seed] integration/deveco/benchmark/scenarios/S8-perf ok
