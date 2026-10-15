#!/usr/bin/env bash
# read-CCR 完成度影响性 A/B：同一 backlog 跑两臂（关/开），比较完成度与收益。
# 判定：完成度（verify + 独立验收）任一下降 → 一票否决，不保留 read-CCR。
#
# 用法：
#   bash ab-read-ccr.sh                      # 默认跑 backlog 2..8
#   bash ab-read-ccr.sh --from 2 --count 8
#   bash ab-read-ccr.sh --rounds "2 3 4"     # 指定轮次
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

FROM=2
COUNT=8
ROUNDS=""
while [ $# -gt 0 ]; do
  case "$1" in
    --from) FROM="$2"; shift 2;;
    --count) COUNT="$2"; shift 2;;
    --rounds) ROUNDS="$2"; shift 2;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done
[ -z "$ROUNDS" ] && ROUNDS="$(seq "$FROM" "$COUNT")"

run_arm() { # arm: off|on
  local arm="$1"
  local work="$AB_WORK/$arm"
  echo "############ ARM $arm ############"
  rm -rf "$work"; mkdir -p "$work"
  TSK_CASE_WORK="$work" bash ./gen-seed.sh >/dev/null 2>&1 || { echo "gen-seed 失败"; return 1; }
  local flag=""
  [ "$arm" = on ] && flag="1"
  for i in $ROUNDS; do
    TSK_CASE_WORK="$work" TSK_READ_CCR="$flag" bash ./run-persistent.sh --only "$i" 2>&1 | grep -E "^\[iter|saved " | sed "s/^/[$arm] /"
  done
  # 独立验收（agent 无关，按 SPEC 契约打 HTTP）
  local acc="?"
  node ./acceptance/run.mjs "$work/seed" >"$work/acceptance.txt" 2>&1 && acc="pass" || acc="FAIL"
  echo "[$arm] 独立验收: $acc"
}

run_arm off
run_arm on

echo "############ 对比 ############"
node - "$HERE" "$ROUNDS" <<'EOF'
const base = process.env.AB_WORK || ".work-ab"
const [here, roundsStr] = process.argv.slice(2)
const fs = require("fs"), path = require("path")
const rounds = roundsStr.split(/\s+/).filter(Boolean).map(Number)
const rows = []
for (const arm of ["off", "on"]) {
  for (const i of rounds) {
    const f = path.join(here, base, arm, "runs", `iter-${i}.metrics.json`)
    if (!fs.existsSync(f)) { rows.push({ arm, i, missing: true }); continue }
    rows.push({ arm, i, ...JSON.parse(fs.readFileSync(f, "utf8")) })
  }
}
const sum = (arm, k) => rows.filter((r) => r.arm === arm && !r.missing).reduce((a, r) => a + (r[k] || 0), 0)
const done = (arm) => rows.filter((r) => r.arm === arm && r.verify_pass).length
const n = rounds.length
console.log("轮次         :", rounds.join(","))
console.log("完成度 off/on:", `${done("off")}/${n}`, "/", `${done("on")}/${n}`)
console.log("saved  off/on:", sum("off", "saved_tokens"), "/", sum("on", "saved_tokens"))
console.log("净收益 off/on:", sum("off", "net_tokens"), "/", sum("on", "net_tokens"))
const verdict = done("on") < done("off") ? "❌ 完成度下降 → 否决 read-CCR"
  : sum("on", "saved_tokens") <= sum("off", "saved_tokens") ? "⚠️ 完成度持平但收益没提升 → 不值得留"
  : "✅ 完成度持平且收益提升 → 可保留"
console.log("判定        :", verdict)
EOF
