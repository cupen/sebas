#!/usr/bin/env bash
# 沙箱 webui 联调环境（一次性目录 + 独立端口，绝不触碰真实 ~/.sebas 与 9797）。
#
# 用法：
#   bash scripts/test_webui_sandbox.sh            # 前台运行，Ctrl-C 退出并清理
#   SANDBOX_PORT=9881 bash scripts/test_webui_sandbox.sh
#   SANDBOX_AUTH=1 bash scripts/test_webui_sandbox.sh   # 打开鉴权（admin/admin）
#
# 鉴权默认关闭（add-webui-auth-switch：测试环境免登录联调）；SANDBOX_AUTH=1
# 时写 auth = true 并创建统一测试账户 admin / admin，用于验证登录流。
# 前端联调：operator 照常跑 pnpm run dev（127.0.0.1:5273）；本脚本只负责后端。
# 验收后脚本退出时会 SIGTERM webui 并删除沙箱目录。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/debug/sebas"
PORT="${SANDBOX_PORT:-9879}"
AUTH="${SANDBOX_AUTH:-0}"
# TOML 布尔字面量（SANDBOX_AUTH 用 0/1 便于 shell 传参）。
AUTH_TOML="false"; [ "$AUTH" = "1" ] && AUTH_TOML="true"
WORK="${SANDBOX_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/sebas-webui-itest.XXXXXX")}"

if [ ! -x "$BIN" ]; then
  echo "error: $BIN 不存在，先 cargo build" >&2
  exit 1
fi

mkdir -p "$WORK/media" "$WORK/acp"
cat > "$WORK/config.toml" <<EOF
[feishu]
app_id = ""
app_secret = ""

[router]
state_file = "$WORK/state.json"

[media]
download_dir = "$WORK/media"

[acp.claude]
path = "claude"
args = []
sessions_dir = "$WORK/acp"
work_dir = "$WORK"

[watchdog.core]
enabled = false
channel_path = "$WORK/core.sock"

[watchdog.webui]
enabled = true
host = "127.0.0.1"
port = $PORT
auth = $AUTH_TOML
EOF

cleanup() {
  if [ -n "${WEBUI_PID:-}" ] && kill -0 "$WEBUI_PID" 2>/dev/null; then
    kill "$WEBUI_PID" 2>/dev/null || true
    wait "$WEBUI_PID" 2>/dev/null || true
  fi
  # 优雅退出的 state dump / socket 清理由进程自身完成；目录整体删除。
  rm -rf "$WORK"
  echo "sandbox cleaned: $WORK"
}
trap cleanup EXIT INT TERM

# 测试账户仅在显式开鉴权时创建（admin / admin）。
if [ "$AUTH" = "1" ]; then
  SEBAS_WEBUI_AUTH_FILE="$WORK/webui-auth.json" \
    "$BIN" webui-passwd --user admin --password-stdin <<< "admin"
fi

SEBAS_CORE_SECRET=fake \
SEBAS_STATE_FILE="$WORK/state.json" \
SEBAS_GATEWAY_PROVIDER_OVERLAY="$WORK/provider-overlay.json" \
SEBAS_WEBUI_AUTH_FILE="$WORK/webui-auth.json" \
  "$BIN" webui -c "$WORK/config.toml" &
WEBUI_PID=$!

for _ in $(seq 1 30); do
  if curl -fsS -o /dev/null "http://127.0.0.1:$PORT/health" 2>/dev/null; then
    break
  fi
  sleep 0.5
done

if [ "$AUTH" = "1" ]; then
  echo "### webui 沙箱就绪：http://127.0.0.1:$PORT/  （鉴权开启，admin / admin）"
else
  echo "### webui 沙箱就绪：http://127.0.0.1:$PORT/  （鉴权关闭，免登录）"
fi
echo "### 凭据文件：$WORK/webui-auth.json    日志与状态均在 $WORK"
echo "### Ctrl-C 退出并清理"
wait "$WEBUI_PID"
