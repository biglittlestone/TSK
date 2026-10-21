#!/usr/bin/env bash
# 常驻优化循环：每轮独立会话跑一条 backlog 任务 → 验收 → git 提交 → 记录 TSK 收益增量。
#
# 用法：
#   bash run-persistent.sh                 # 按 backlog.json 顺序跑前 N（默认 8）轮
#   bash run-persistent.sh --count 3       # 只跑前 3 轮
#   bash run-persistent.sh --only 2        # 只跑第 2 轮
#   bash run-persistent.sh --model deveco/GLM-5.1
#
# 环境变量：
#   TSK_READ_CCR=1                  read-CCR（默认开；设 0/空 关闭——A/B 的 off 臂）
#   TSK_GLM_DAILY_BUDGET=50000000   GLM 免费额度/日（默认 50M）；达上限自动停
#   TSK_GLM_RETRIES=2               通道过载时重试次数（默认 2；退避 60s/240s）
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="${TSK_CASE_WORK:-$HERE/.work}"
# 归一为绝对路径：相对路径 + cwd 漂移会让 seed 内的 git 提交落进上级仓库（历史教训，勿删）
case "$WORK" in /*) ;; *) WORK="$HERE/$WORK" ;; esac
SEED="$WORK/seed"
RUNS="$WORK/runs"
JOURNAL="$SEED/.tsk/injections.jsonl"
cd "$HERE"  # 之后 node 用相对路径（Git Bash 的 /e/... 绝对路径 node 解析不了）
COUNT=8
FROM=1
ONLY=""
MODEL="${TSK_CASE_MODEL:-deveco/GLM-5.1}"

# —— read-CCR 口径固化 ——
# 主案例库 22 轮起一直是「开」着 read-CCR 跑的，但该变量从未持久化；续跑漏带会丢掉
# read 通道 ~27% 的 saved，新旧轮次不可比（2026-10-16 实测教训）。这里在「未设置」时
# 默认开；显式设为空串/0（如 A/B 的 off 臂）则保持关闭，不破坏分臂。
if [ -z "${TSK_READ_CCR+x}" ]; then export TSK_READ_CCR=1; fi
# —— GLM 免费通道硬约束：50M tok/日 ——
# 超预算或通道过载时优雅停止，且不写 metrics（保持可续跑）。
GLM_DAILY_BUDGET="${TSK_GLM_DAILY_BUDGET:-50000000}"

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

# ---- 工作区：先生成（新目录）或沿用；再强校验 .git（任何 git 操作前必须成立，防 add -A 走上库）----
[ -d "$SEED/.git" ] || bash "$HERE/gen-seed.sh" >/dev/null || true
[ -d "$SEED/.git" ] || { echo "[fatal] $SEED 不是 git 仓库（gen-seed 失败？）" >&2; exit 4; }
mkdir -p "$RUNS"
touch "$JOURNAL" 2>/dev/null || true   # 追加式：每轮按当前行数取增量，不截断已有记录

# ---- 模拟器看门狗（兜底；插件已拦一层）----
if [ -f "$HERE/../benchmark/emulator-watchdog.sh" ]; then
  bash "$HERE/../benchmark/emulator-watchdog.sh" & WD=$!
  trap '[ -n "${WD:-}" ] && kill $WD 2>/dev/null' EXIT
fi

prompt_of() { node -e "const b=require('./backlog.json');const t=b.find(x=>x.id==Number(process.argv[1]));process.stdout.write(t?t.prompt:'')" "$1"; }
title_of()  { node -e "const b=require('./backlog.json');const t=b.find(x=>x.id==Number(process.argv[1]));process.stdout.write(t?t.title:'')" "$1"; }

# 今日已消耗的会话 input tokens（按 metrics 文件 mtime 归日）——用于 GLM 50M/日 免费额度闸门。
today_used() {
  node -e '
    const fs=require("fs"),path=require("path");
    const dir=process.argv[1]; if(!fs.existsSync(dir)){process.stdout.write("0");process.exit(0)}
    const d=new Date(); d.setHours(0,0,0,0);
    let sum=0;
    for(const f of fs.readdirSync(dir)){
      if(!/^iter-\d+\.metrics\.json$/.test(f))continue;
      const p=path.join(dir,f);
      if(fs.statSync(p).mtimeMs < d.getTime())continue;
      try{sum+=Number(JSON.parse(fs.readFileSync(p,"utf8")).session_input_tokens)||0}catch{}
    }
    process.stdout.write(String(sum));
  ' "$RUNS"
}

# 识别 GLM 通道错误：quota（额度耗尽→硬停）/ overload（过载→退避重试）/ 空（正常）。
# 只认显式措辞；事件流(json)比 stderr 更保守，避免把模型正文/业务数据误判。
detect_glm_error() {
  local err="$1" json="$2"
  printf '%s' "$err" | grep -qiE 'quota|额度|超额|daily limit|usage limit|exhaust|余额' && { echo quota; return; }
  printf '%s' "$err" | grep -qiE 'overload|过载|繁忙|rate.?limit|too many requests|temporarily unavailable|service unavailable|503|502' && { echo overload; return; }
  printf '%s' "$json" | grep -qiE 'quota exceeded|insufficient quota|usage limit|daily limit reached' && { echo quota; return; }
  printf '%s' "$json" | grep -qiE 'overloaded|rate.?limit|too many requests|temporarily unavailable' && { echo overload; return; }
  echo ""
}

# 硬性收尾：要求 agent 在会话内跑验收（后端测试 + 前端构建）并贴出输出。
# 否则 agent 常常改完就宣布完成，构建只在循环外发生 → 会话内没有大输出，TSK 无从压缩。
VERIFY_TAIL=' 最后必须运行 `bash verify.sh`（它会跑后端 bun test 和前端 hvigor 构建），把命令与输出贴出来；不通过就继续修到通过。不要提问。'

iterations() { if [ -n "$ONLY" ]; then echo "$ONLY"; else seq "$FROM" "$COUNT"; fi; }

for i in $(iterations); do
  # 可续跑：已有该轮指标则跳过（被系统收割后重跑不丢进度）
  [ -f "$RUNS/iter-$i.metrics.json" ] && { echo "[iter $i] 已有结果，跳过"; continue; }
  PROMPT="$(prompt_of "$i")"
  TITLE="$(title_of "$i")"
  [ -z "$PROMPT" ] && { echo "[iter $i] backlog 无此条，跳过"; continue; }
  PROMPT="$PROMPT$VERIFY_TAIL"

  # 每日免费额度闸门：达到 GLM 50M tok/日 上限就停（硬约束）
  USED="$(today_used)"
  if [ "$USED" -ge "$GLM_DAILY_BUDGET" ]; then
    echo "[budget] 今日已用 ${USED}/${GLM_DAILY_BUDGET} tok，达到 GLM 免费额度上限，停止。明日续跑（已完成轮次自动跳过）。"
    break
  fi
  echo "[budget] 今日已用 ${USED}/${GLM_DAILY_BUDGET} tok（TSK_READ_CCR=${TSK_READ_CCR:-0}）"

  echo "==================== iter $i: $TITLE ===================="
  QUOTA_STOP=0; OVERLOAD_STOP=0; ATTEMPT=0
  while :; do
    ATTEMPT=$((ATTEMPT + 1))
    LINES_BEFORE="$(wc -l < "$JOURNAL" 2>/dev/null || echo 0)"

    # 1) 跑一轮（新会话；单轮上限 60 分钟）
    START=$(date +%s)
    ( cd "$SEED" && timeout 3600 deveco run "$PROMPT" --format json --agent goal --model "$MODEL" --auto --title "opt-$i" ) \
      > "$RUNS/iter-$i.json" 2> "$RUNS/iter-$i.err"
    RC=$?
    END=$(date +%s)
    echo "[iter $i] deveco rc=$RC 用时 $((END-START))s（第 $ATTEMPT 次）"

    # 额度耗尽/过载：优雅停止或退避重试；不写 metrics（保持可续跑）
    GLM_ERR="$(detect_glm_error "$(tail -c 8000 "$RUNS/iter-$i.err" 2>/dev/null)" "$(tail -c 8000 "$RUNS/iter-$i.json" 2>/dev/null)")"
    [ -z "$GLM_ERR" ] && break
    if [ "$GLM_ERR" = quota ]; then
      echo "[quota] 检测到 GLM 免费额度耗尽（今日已用 $(today_used)/${GLM_DAILY_BUDGET} tok）。本轮不计分，停止；明日重跑本命令即可续跑。"
      QUOTA_STOP=1; break
    fi
    if [ "$ATTEMPT" -gt "${TSK_GLM_RETRIES:-2}" ]; then
      echo "[overload] GLM 通道连续 $ATTEMPT 次过载，停止。本轮不计分；稍后重跑本命令即可续跑。"
      OVERLOAD_STOP=1; break
    fi
    BACKOFF=$((60 * ATTEMPT * ATTEMPT))
    echo "[overload] GLM 通道过载，${BACKOFF}s 后重试（第 $ATTEMPT 次）..."
    sleep "$BACKOFF"
  done
  if [ "$QUOTA_STOP" = 1 ] || [ "$OVERLOAD_STOP" = 1 ]; then break; fi

  # 2) 会话导出（token 口径）
  SID="$(node collect.mjs --session-of "$RUNS/iter-$i.json" 2>/dev/null || true)"
  if [ -n "$SID" ]; then
    ( cd "$SEED" && deveco export "$SID" ) > "$RUNS/export-$i.json" 2>/dev/null || true
  fi

  # 3) 验收（deveco 失败/超时 → 强制验收失败，防止「无改动→verify 放行」的假阳性）
  VERIFY_OUT="$(bash "$SEED/verify.sh" 2>&1 | tail -1)"
  bash "$SEED/verify.sh" >/dev/null 2>&1; VERIFY_RC=$?
  if [ "$RC" != 0 ]; then VERIFY_RC=1; VERIFY_OUT="deveco rc=$RC（会话失败/被挡），$VERIFY_OUT"; fi
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
