#!/usr/bin/env bash
# 任务级验收（按目录存在性，与 git 状态无关）：
#   server/ 存在 → 跑 `bun test` 必须全绿
#   app/    存在 → 跑 hvigor 构建并确认产出 hap
# 退出码 0 = 该轮完成度通过。
set -u
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

STUDIO="${DEVECO_STUDIO:-D:/DevEco Studio}"
# Git Bash 下 PATH 必须是 POSIX 形式，否则 hvigor 起不来 java（spawn java ENOENT）
STUDIO_POS="$(cygpath -u "$STUDIO" 2>/dev/null || echo "$STUDIO")"
export DEVECO_SDK_HOME="$STUDIO" DEVECO_HOME="$STUDIO" JAVA_HOME="$STUDIO/jbr"
export PATH="$STUDIO_POS/jbr/bin:$STUDIO_POS/tools/hvigor/bin:$STUDIO_POS/tools/ohpm/bin:$PATH"

OK=1
RESULTS=""

# ---- 后端 ----
if [ -d server ]; then
  if (cd server && bun test >/dev/null 2>&1); then
    RESULTS="$RESULTS server:pass"
  else
    RESULTS="$RESULTS server:FAIL"
    OK=0
  fi
else
  RESULTS="$RESULTS server:missing"
  OK=0
fi

# ---- 前端 ----
if [ -d app ]; then
  # 首次构建前装依赖（oh_modules 被 gitignore，不在 git 历史里）
  if [ ! -d app/oh_modules ] && [ -f app/oh-package.json5 ]; then
    (cd app && ohpm install >/dev/null 2>&1) || true
  fi
  if (cd app && "$STUDIO/tools/hvigor/bin/hvigorw.bat" assembleHap --no-daemon >/dev/null 2>&1) \
     && ls app/entry/build/default/outputs/default/*.hap >/dev/null 2>&1; then
    RESULTS="$RESULTS app:pass"
  else
    RESULTS="$RESULTS app:FAIL"
    OK=0
  fi
else
  RESULTS="$RESULTS app:missing"
  OK=0
fi

echo "verify:${RESULTS}"
exit $(( 1 - OK ))
