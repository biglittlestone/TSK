#!/usr/bin/env bash
# integration/deveco/benchmark/scenarios/S9-tdd —— $1 根基线
set -euo pipefail
cd "${1:?workdir}"
printf '%s\n' 'module.exports={add:(a,b)=>a+b};' > calc.js; printf '%s\n' 'const a=require("node:assert/strict");const C=require("./calc");for(let i=0;i<60;i++)require("node:test").test("add "+i,()=>a.strictEqual(C.add(i,i),i*2));' > calc.test.js
echo [seed] integration/deveco/benchmark/scenarios/S9-tdd ok
