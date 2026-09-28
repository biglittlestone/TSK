#!/usr/bin/env bash
# S3-scaffold: 脚手架+依赖+think-in-code 汇总 —— 在 $1 根生成确定性基线（真实 provisioning 由场景运行前置脚本补齐）
set -euo pipefail
cd "${1:?workdir}"
echo "[seed] S3-scaffold 基线就绪（请按 DEVECO-BENCHMARKS.md 的运行前置补真实 workspace）"
