#!/usr/bin/env bash
# 常驻优化循环：每轮独立会话跑一条 backlog 任务 → 验收 → git 提交 → 记录 TSK 收益增量。
#
# 用法：
#   bash run-persistent.sh                 # 按 backlog.json 顺序跑前 N（默认 8）轮
#   bash run-persistent.sh --count 3       # 只跑前 3 轮
#   bash run-persistent.sh --only 2        # 只跑第 2 轮
#   bash run-persistent.sh --model deveco/GLM-5.1
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"                       # 之后一律用相对路径（Git Bash 的 /e/... 路径 node 解析不了）

WORK="${TSK_CASE_WORK:-.work}"
SEED="$WORK/seed"
RUNS="$WORK/runs"
JOURNAL="$SEED/.tsk/injections.jsonl"
COUNT=8
FROM=1
ONLY=""
MODEL="${TSK_CASE_MODEL:-deveco/GLM-5.1}"

while [ $# -gt 0 ]; do
  case "$1" in
    --count) COUNT="$2"; shift 2;;
    --from)  FROM="$2"; shift 2;;
    --only)  ONLY="$2"; shift 2;;
    --model) MODEL="$2"; shift 2;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done

# ---- 环境：DevEco 工具链 + tsk 在 PATH ----
STUDIO="${DEVECO_STUDIO:-D:/DevEco Studio}"
# Git Bash 下 PATH 必须是 POSIX 形式，否则 hvigor 起不来 java（spawn java ENOENT）
STUDIO_POS="$(cygpath -u "$STUDIO" 2>/dev/null || echo "$STUDIO")"
export DEVECO_SDK_HOME="$STUDIO" DEVECO_HOME="$STUDIO" JAVA_HOME="$STUDIO/jbr"
export PATH="$HOME/bin:$STUDIO_POS/jbr/bin:$STUDIO_POS/tools/hvigor/bin:$STUDIO_POS/tools/ohpm/bin:$PATH"
# journal 必须落在工作区内，才能按轮归因
unset TSK_JOURNAL_DIR

command -v tsk >/dev/null || { echo "tsk 不在 PATH" >&2; exit 3; }
command -v deveco >/dev/null || { echo "deveco 不在 PATH" >&2; exit 3; }

# ---- 工作区 ----
[ -d "$SEED/.git" ] || bash ./gen-seed.sh >/dev/null
mkdir -p "$RUNS"
touch "$JOURNAL" 2>/dev/null || true   # 追加式：每轮按当前行数取增量，不截断已有记录

# ---- 模拟器看门狗（兜底；插件已拦一层）----
if [ -f "../benchmark/emulator-watchdog.sh" ]; then
  bash ../benchmark/emulator-watchdog.sh & WD=$!
  trap '[ -n "${WD:-}" ] && kill $WD 2>/dev/null' EXIT
fi

prompt_of() { node -e "const b=require('./backlog.json');const t=b.find(x=>x.id==Number(process.argv[1]));process.stdout.write(t?t.prompt:'')" "$1"; }
title_of()  { node -e "const b=require('./backlog.json');const t=b.find(x=>x.id==Number(process.argv[1]));process.stdout.write(t?t.title:'')" "$1"; }

# 硬性收尾：要求 agent 在会话内跑验收（后端测试 + 前端构建）并贴出输出。
# 否则 agent 常常改完就宣布完成，构建只在循环外发生 → 会话内没有大输出，TSK 无从压缩。
VERIFY_TAIL=' 最后必须运行 `bash verify.sh`（它会跑后端 bun test 和前端 hvigor 构建），把命令与输出贴出来；不通过就继续修到通过。不要提问。'

iterations() { if [ -n "$ONLY" ]; then echo "$ONLY"; else seq "$FROM" "$COUNT"; fi; }

for i in $(iterations); do
  PROMPT="$(prompt_of "$i")"
  TITLE="$(title_of "$i")"
  [ -z "$PROMPT" ] && { echo "[iter $i] backlog 无此条，跳过"; continue; }
  PROMPT="$PROMPT$VERIFY_TAIL"

  echo "==================== iter $i: $TITLE ===================="
  LINES_BEFORE="$(wc -l < "$JOURNAL" 2>/dev/null || echo 0)"

  # 1) 跑一轮（新会话；单轮上限 60 分钟）
  START=$(date +%s)
  ( cd "$SEED" && timeout 3600 deveco run "$PROMPT" --format json --agent goal --model "$MODEL" --auto --title "opt-$i" ) \
    > "$RUNS/iter-$i.json" 2> "$RUNS/iter-$i.err"
  RC=$?
  END=$(date +%s)
  echo "[iter $i] deveco rc=$RC 用时 $((END-START))s"

  # 2) 会话导出（token 口径）
  SID="$(node collect.mjs --session-of "$RUNS/iter-$i.json" 2>/dev/null || true)"
  if [ -n "$SID" ]; then
    ( cd "$SEED" && deveco export "$SID" ) > "$RUNS/export-$i.json" 2>/dev/null || true
  fi

  # 3) 验收
  VERIFY_OUT="$(bash "$SEED/verify.sh" 2>&1 | tail -1)"
  bash "$SEED/verify.sh" >/dev/null 2>&1; VERIFY_RC=$?
  echo "[iter $i] $VERIFY_OUT (rc=$VERIFY_RC)"

  # 4) 提交（保留可回滚节点）
  ( cd "$SEED" && git add -A && git commit -q -m "opt-$i: $TITLE" ) 2>/dev/null || true

  # 5) 归因
  node collect.mjs --iter "$i" --journal "$JOURNAL" --lines-before "$LINES_BEFORE" \
       --run "$RUNS/iter-$i.json" --export "$RUNS/export-$i.json" --verify-rc "$VERIFY_RC" \
       --title "$TITLE" --seconds "$((END-START))" 2>&1 | tail -6
done

echo "==================== 汇总 ===================="
node collect.mjs --report --runs "$RUNS" --journal "$JOURNAL"
