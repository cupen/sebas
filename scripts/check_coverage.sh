#!/usr/bin/env bash
# 覆盖率门禁（spec §4.3，sebas-nya）。
#
# 用法：cargo llvm-cov --workspace --json --summary-only --output-path cov.json
#       ./scripts/check_coverage.sh cov.json
#
# 阈值现状（2026-07-31，sebas-nya）：
#   router/  ≥ 90%  —— spec 目标，已达到（当前 91.9%）
#   cards.rs ≥ 90%  —— spec 目标，已达到（当前 95.5%）
#   gateway/ ≥ 70%  —— 棘轮下限（sebas-lva.10，Task 10 spec-diff 门禁）。
#                      P0 期合同测试与单测覆盖核心路径；随 P1 全量 contract
#                      测试（sebas-lva.12）补齐，上调至 spec §4.3 目标 80%+。
#   整体     ≥ 65%  —— **棘轮下限**，非 spec 目标。spec §4.3 目标是 80%，
#                      当前 66.4%；缺口集中在 src/run.rs（WS 循环/dispatch）、
#                      install_service、feishu client/media 的 I/O 路径，
#                      依赖 sebas-vw5 的 fixture harness 与 smoke test 补齐。
#                      只许上调、不许下调 —— 覆盖率回退即 CI 红。
set -euo pipefail

json="${1:?usage: check_coverage.sh <coverage.json>}"

OVERALL_MIN=65
ROUTER_MIN=90
CARDS_MIN=90
GATEWAY_MIN=70

overall=$(jq -r '.data[0].totals.lines.percent' "$json")
router=$(jq -r '
  [.data[].files[] | select(.filename | test("/router/src/")) | .summary.lines]
  | (map(.covered) | add) as $c | (map(.count) | add) as $t
  | if $t == 0 then 100 else ($c * 100 / $t) end' "$json")
cards=$(jq -r '
  [.data[].files[] | select(.filename | endswith("feishu/src/cards.rs")) | .summary.lines]
  | (map(.covered) | add) as $c | (map(.count) | add) as $t
  | if $t == 0 then 100 else ($c * 100 / $t) end' "$json")
gateway=$(jq -r '
  [.data[].files[] | select(.filename | test("/gateway/src/")) | .summary.lines]
  | (map(.covered) | add) as $c | (map(.count) | add) as $t
  | if $t == 0 then 100 else ($c * 100 / $t) end' "$json")

fail=0
check() { # check <name> <actual> <min>
  if awk -v a="$2" -v m="$3" 'BEGIN{ exit !(a+0 < m+0) }'; then
    printf '✗ %-10s %6.2f%% < %d%%\n' "$1" "$2" "$3"
    fail=1
  else
    printf '✓ %-10s %6.2f%% ≥ %d%%\n' "$1" "$2" "$3"
  fi
}

check "overall" "$overall" "$OVERALL_MIN"
check "router/" "$router" "$ROUTER_MIN"
check "cards.rs" "$cards" "$CARDS_MIN"
check "gateway/" "$gateway" "$GATEWAY_MIN"

if [ "$fail" -ne 0 ]; then
  echo "coverage gate failed (spec §4.3; overall 棘轮下限见脚本头注释)" >&2
fi
exit "$fail"
