#!/usr/bin/env bash
# ============================================================================
# TSK scenario benchmark — on/off real-model multi-turn runner.
#
# For ONE scenario: OFF (TSK disabled) then ON (TSK enabled), each a fresh
# `claude -p` in the scenario workspace. Captures the full json stream,
# measures total input tokens, runs success.sh, emits a comparison report.
#
# USAGE:  run-onoff.sh <scenario-dir> <results-dir>  [on|off]
#   <scenario-dir>  e.g. scenarios/01-calc-web  (must contain task.md, success.sh)
#   <results-dir>   where on.json / off.json / ...  are written
#   [on|off]        optional: run only one mode (for a pilot). Default: both.
# ============================================================================
set -u
SCEN="${1:?scenario dir required}"
RESULTS="${2:?results dir required}"
ONLY="${3:-both}"
SCEN="$(cd "$SCEN" && pwd)"
RESULTS="$(cd "$RESULTS" 2>/dev/null && pwd || echo "$RESULTS")"
TASK="$(cat "$SCEN/task.md")"
OUT_TAG="$RESULTS"
mkdir -p "$OUT_TAG"

SELF="$(cd "$(dirname "$0")" && pwd)"          # scenarios/ (contains measure.mjs)
MEASURE="node $SELF/measure.mjs"

ensure_plugin() { # arg: on | off
  if [ "$1" = on ]; then claude plugin enable tsk@skills-dir >/dev/null 2>&1;
  else claude plugin disable tsk@skills-dir >/dev/null 2>&1; fi
}

reset_solution() { rm -rf "$SCEN/solution"; mkdir -p "$SCEN/solution"; }

run_mode() {
  local mode="$1"
  ensure_plugin "$mode"
  ensure_plugin "$mode"   # belt+suspenders: confirm state before launch
  reset_solution         # each mode starts from the same empty solution
  local cap="$OUT_TAG/$mode.json"
  pushd "$SCEN" >/dev/null
  echo "  [run] $mode — claude -p (may take a while)"
  # OFF baseline strips tsk's bin dir from PATH so the sandbox CLI is genuinely absent
  if [ "$mode" = off ]; then
    export PATH=$(printf '%s' "$PATH" | tr ':' '\n' | grep -vE 'Administrator/bin|skills/tsk/bin' | paste -sd: -)
    command -v tsk >/dev/null 2>&1 && echo "  [warn] tsk still on PATH in OFF" || echo "  [off] tsk NOT on PATH"
  fi
  claude -p "$TASK" --output-format stream-json --verbose --dangerously-skip-permissions --max-turns 30 \
    > "$cap" 2> "$OUT_TAG/$mode.stderr.log"
  local rc=$?
  popd >/dev/null
  # verdict
  local verdict="no-solution"
  if [ -f "$SCEN/solution/index.html" ] || bash "$SCEN/success.sh" >/dev/null 2>&1; then
    verdict="PASS"; else verdict="FAIL"; fi
  # capture artifacts
  if [ -d "$SCEN/solution" ]; then cp -r "$SCEN/solution" "$OUT_TAG/$mode.solution"; fi
  echo "  [done] $mode rc=$rc verdict=$verdict"
}

# baseline pass check (empty solution dir must exist in seed)
for m in ${ONLY}; do
  case "$m" in
    on)  echo "===== ON  (TSK enabled) : $SCEN ====="; run_mode on;;
    off) echo "===== OFF (TSK disabled): $SCEN ====="; run_mode off;;
    *) echo "unknown mode: $m"; exit 2;;
  esac
done

# always restore TSK so the interactive session stays intact
ensure_plugin on
echo "[plugin] TSK restored"

# summary
if [ -f "$OUT_TAG/on.json" ] && [ -f "$OUT_TAG/off.json" ]; then
  echo "===== SUMMARY : $SCEN ====="
  echo "  OFF: $(node $SELF/measure.mjs "$OUT_TAG/off.json" | grep -E '"turns"|"total_input_tokens"|"cost_usd"' | tr '\n' ' ')"
  echo "  ON : $(node $SELF/measure.mjs "$OUT_TAG/on.json"  | grep -E '"turns"|"total_input_tokens"|"cost_usd"' | tr '\n' ' ')"
fi