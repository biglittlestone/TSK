#!/usr/bin/env bash
# 成功 = solution/overview.md 存在且覆盖全部 10 个域名
cd "$(dirname "$0")"
[ -f solution/overview.md ] || { echo "FAIL: no solution/overview.md"; exit 1; }
DOMAINS="总体架构 数据模型 认证与权限 订单子系统 库存子系统 支付与结算 消息与通知 报表与统计 安全与合规 运维与监控"
missing=0
for d in $DOMAINS; do grep -qF "$d" solution/overview.md || { echo "MISSING domain: $d"; missing=1; }
done
[ "$missing" = 0 ] || exit 1
echo "OK: overview.md 覆盖全部 10 个域名"