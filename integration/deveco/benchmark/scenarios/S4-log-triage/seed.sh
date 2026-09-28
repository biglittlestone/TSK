#!/usr/bin/env bash
# S4-log-triage —— 1200 行日志 + parser 测试基线
set -euo pipefail
cd "${1:?workdir}"
node -e 'const f=require("fs");let L=[];for(let i=0;i<1200;i++){L.push(i%17===0?"ERROR build-"+(i%23)+" cannot load mod"+(i%9):"[info] step "+i)}f.writeFileSync("full.log",L.join("\n"))'
printf '%s\n' 'const a=require("node:assert/strict");require("node:test").test("parse",()=>a.strictEqual(1,2));' > parser.test.js
echo "[S4] full.log + parser.test.js"
