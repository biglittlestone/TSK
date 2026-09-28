#!/usr/bin/env bash
# S5-kb-build: 研读5文件产设计文档(沙箱+检索) —— 在 $1 根生成确定性基线（真实 provisioning 由场景运行前置脚本补齐）
set -euo pipefail
cd "${1:?workdir}"
echo "[seed] S5-kb-build 基线就绪（请按 DEVECO-BENCHMARKS.md 的运行前置补真实 workspace）"
