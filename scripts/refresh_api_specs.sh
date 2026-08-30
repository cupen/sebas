#!/usr/bin/env bash
# 刷新 vendored 官方 API 清单（spec-diff 门禁用，Task 10 / sebas-lva.10）。
#
# 用法：./scripts/refresh_api_specs.sh [--cached]
#
# 默认：从 GitHub 实时拉取官方源文件最新版，写入 sebas-gateway/tests/specs/：
#   - openai-openapi.yaml   ← openai/openai-openapi          openapi.yaml
#   - anthropic-api.md      ← anthropics/anthropic-sdk-typescript  api.md
#
# --cached：不联网。从本地 /tmp 种子副本（CI 已预置 / 之前下载过）复制到
# vendor 目标，便于离线环境。/tmp 副本缺失时报错退出。
#
# Accept: application/vnd.github.raw 返回文件原始内容（不经 base64/JSON 包裹）。
set -euo pipefail

SPECS_DIR="$(cd "$(dirname "$0")/.." && pwd)/sebas-gateway/tests/specs"
mkdir -p "$SPECS_DIR"

GH_RAW="Accept: application/vnd.github.raw"

CACHED=0
for arg in "$@"; do
  case "$arg" in
    --cached) CACHED=1 ;;
    -h|--help)
      sed -n '2,/^$/p' "$0" | sed 's/^# \?//'
      exit 0 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

fetch() { # fetch <url> <out_path>
  local url="$1" out="$2"
  if [[ "$CACHED" -eq 1 ]]; then
    local cache="/tmp/$(basename "$out")"
    if [[ -s "$cache" ]]; then
      echo "cached: $cache -> $out"
      cp -f "$cache" "$out"
    else
      echo "error: --cached but $cache missing; run without --cached first" >&2
      exit 1
    fi
    return 0
  fi
  echo "fetch: $url -> $out"
  curl -fsSL -H "$GH_RAW" "$url" -o "$out"
}

# OpenAI 官方 OpenAPI（openai/openai-openapi 仓库根 openapi.yaml）
fetch \
  "https://api.github.com/repos/openai/openai-openapi/contents/openapi.yaml" \
  "$SPECS_DIR/openai-openapi.yaml"

# Anthropic 官方 api.md（anthropic-sdk-typescript 仓库根 api.md）
fetch \
  "https://api.github.com/repos/anthropics/anthropic-sdk-typescript/contents/api.md" \
  "$SPECS_DIR/anthropic-api.md"

echo "done. vendored specs:"
ls -la "$SPECS_DIR"
