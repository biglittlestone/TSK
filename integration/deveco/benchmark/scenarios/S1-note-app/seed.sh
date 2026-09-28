#!/usr/bin/env bash
# S1-note-app: 笔记App:逐页实现+每步 build 排错 —— 在 $1 根生成确定性基线（真实 provisioning 由场景运行前置脚本补齐）
set -euo pipefail
cd "${1:?workdir}"
echo "[seed] S1-note-app 基线就绪（请按 DEVECO-BENCHMARKS.md 的运行前置补真实 workspace）"
