#!/usr/bin/env bash
# 刷新 vendored 官方 API 清单（spec-diff 门禁用，Task 10 / sebas-lva.10）。
#
# 用法：./scripts/refresh_api_specs.sh
#
# 从 GitHub API 拉取官方源文件最新版，写入 gateway/tests/specs/：
#   - openai-openapi.yaml   ← openai/openai-openapi  openapi.yaml
#   - anthropic-api.md      ← anthropics/anthropic-sdk-typescript  api.md
#
# 优先使用本地 /tmp 副本（CI 已预置时）；否则 curl 下载。
# Accept: application/vnd.github.raw 返回文件原始内容（不经 base64/JSON 包裹）。
set -euo pipefail

SPECS_DIR="$(cd "$(dirname "$0")/.." && pwd)/gateway/tests/specs"
mkdir -p "$SPECS_DIR"

GH_RAW="Accept: application/vnd.github.raw"

fetch() { # fetch <url> <out_path>
  local url="$1" out="$2"
  if [ -s "$out" ] && head -c 1 "$out" >/dev/null 2>&1; then
    echo "skip (exists): $out"
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
