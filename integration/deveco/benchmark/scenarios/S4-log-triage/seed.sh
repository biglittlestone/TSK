#!/usr/bin/env bash
# S4-log-triage: 超长日志修真bug(B分节主场景) —— 在 $1 根生成确定性基线（真实 provisioning 由场景运行前置脚本补齐）
set -euo pipefail
cd "${1:?workdir}"
echo "[seed] S4-log-triage 基线就绪（请按 DEVECO-BENCHMARKS.md 的运行前置补真实 workspace）"
