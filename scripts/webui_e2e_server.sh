#!/usr/bin/env bash
# Browser-e2e 沙箱装配（一次性目录 + 独立端口，绝不触碰真实 ~/.sebas 与 9797）。
# 被 playwright webServer 以 `bash scripts/webui_e2e_server.sh` 启动（cwd = 仓库根），
# 本脚本按 AGENTS.md 调试菜谱写 config 并拉起 `sebas core --router --debug --webui`。
#
# 环境开关：
#   E2E_AUTH=1    鉴权开形态（端口 9898，admin/admin，凭据写沙箱内 auth 文件）
#   E2E_PORT=...  覆盖端口（默认 9899；E2E_AUTH=1 时默认 9898）
#   E2E_KEEP=1    退出时不删沙箱目录（调试）
#   E2E_REUSE=1   （配合 E2E_KEEP）复用已保留的沙箱：端口已被占用则直接复用
#
# 退出行为：SIGTERM 后端 → 若目录内有 .tests-failed 标记（keep-on-fail reporter
# 在用例失败时写入）则保留现场并打印路径，否则删除整个沙箱目录。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${E2E_PORT:-$([ "${E2E_AUTH:-0}" = "1" ] && echo 9898 || echo 9899)}"
AUTH="${E2E_AUTH:-0}"
AUTH_TOML="false"; [ "$AUTH" = "1" ] && AUTH_TOML="true"

# --- 平台适配（design D6） ---------------------------------------------------
# Windows（msys/Git Bash）下：二进制名带 .exe；MSYS 参数级路径转换帮不到文件
# 内容，写进 config.toml 的路径必须先经 cygpath -w 转成 Windows 形态；named
# pipe 全名 256 字符上限 → 沙箱目录名取短。
IS_WINDOWS=0
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) IS_WINDOWS=1 ;; esac
EXE=""; [ "$IS_WINDOWS" = "1" ] && EXE=".exe"
BIN="$ROOT/target/debug/sebas$EXE"
FAKE="$ROOT/target/debug/fake-claude$EXE"

if [ ! -x "$BIN" ]; then
  echo "error: $BIN 不存在，先 cargo build --bin sebas --bin fake-claude" >&2
  exit 1
fi
if [ ! -x "$FAKE" ]; then
  echo "error: $FAKE 不存在，先 cargo build --bin fake-claude" >&2
  exit 1
fi

scene_dir() {
  if [ "$IS_WINDOWS" = "1" ]; then
    # 短名（named pipe 全名 ≤256），基于 %TEMP%
    local base="${TEMP:-C:/Windows/Temp}"
    base="${base//\\//}"; base="${base%/}"
    printf '%s/sbe2e%s' "$base" "$(date +%s%N | tail -c 7)"
  else
    mktemp -d "${TMPDIR:-/tmp}/sbe2e.XXXXXX"
  fi
}

to_cfg_path() {
  # config.toml 内容里的路径：Windows 下转回 Windows 形态
  if [ "$IS_WINDOWS" = "1" ]; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

# 复用模式：端口已监听 → 视为已保留现场在服务，直接交还给 playwright。
if [ "${E2E_REUSE:-0}" = "1" ] && [ -f "${E2E_SCENE_FILE:-}" ]; then
  SCENE_FILE="${E2E_SCENE_FILE}"
  WORK="$(cat "$SCENE_FILE" 2>/dev/null || true)"
  if [ -n "$WORK" ] && curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
    echo "[e2e] reusing existing sandbox on port $PORT (dir: $WORK)"
    sleep infinity
  fi
  # 复用条件不满足 → 落回全新沙箱
  unset WORK SCENE_FILE
fi

WORK="$(scene_dir)"
mkdir -p "$WORK/media" "$WORK/acp" "$WORK/work"
# 场景指针：keep-on-fail reporter 由此得知沙箱目录（保留/排障用）。
SCENE_FILE="${E2E_SCENE_FILE:-${TMPDIR:-/tmp}/sebas-webui-e2e-scene-$PORT}"
printf '%s' "$WORK" > "$SCENE_FILE"

# --- config.toml（AGENTS.md 调试菜谱键位） -----------------------------------
W_CFG="$(to_cfg_path "$WORK")"
W_FAKE="$(to_cfg_path "$FAKE")"
cat > "$WORK/config.toml" <<EOF
[feishu]
enabled = false

[acp.agents.claude]
driver = "claude"
path = "$W_FAKE"
sessions_dir = "$W_CFG/claude-sessions"
work_dir = "$W_CFG/work"

[dispatch]
state_file = "$W_CFG/sessions.json"

[media]
download_dir = "$W_CFG/downloads"

[watchdog.core]
channel_path = "$W_CFG/core-channel.sock"

[watchdog.webui]
enabled = false
auth = $AUTH_TOML

# router validate 要求 ≥1 个带 base_url 的 provider；debug 模式下 `test`
# provider 由 --debug 注入，这个哑 provider 从不外拨。
[provider.anthropic]
api_key = "sk-sandbox-dummy"

[router]
provider_overlay = "$W_CFG/providers.json"
usage_file = "$W_CFG/router-usage.jsonl"
EOF

# --- auth 形态：沙箱内凭据文件 + admin/admin --------------------------------
if [ "$AUTH" = "1" ]; then
  SEBAS_WEBUI_AUTH_FILE="$WORK/webui-auth.json" \
    "$BIN" webui-passwd --user admin --password-stdin <<< "admin"
fi

cleanup() {
  if [ -n "${CORE_PID:-}" ] && kill -0 "$CORE_PID" 2>/dev/null; then
    kill "$CORE_PID" 2>/dev/null || true
    wait "$CORE_PID" 2>/dev/null || true
  fi
  if [ -f "$WORK/.tests-failed" ] || [ "${E2E_KEEP:-0}" = "1" ]; then
    # 保留现场时连场景指针一起保留，供 E2E_REUSE=1 复用。
    echo "[e2e] sandbox scene kept at: $WORK (core.log inside)"
  else
    rm -rf "$WORK"
    rm -f "$SCENE_FILE"
    echo "[e2e] sandbox cleaned: $WORK"
  fi
}
trap cleanup EXIT
trap 'exit 0' INT TERM

# --- 起被测后端：core --router --debug --webui（单进程调试形态） --------------
# SEBAS_PROJECTS_PATH：webui 的文件回退项目注册表（state store 引擎在 detached
# 组合后端下不可达时读它）——必须重定向进沙箱，否则会读写真实 ~/.sebas/projects.json。
SEBAS_CORE_SECRET=fake \
SEBAS_STATE_DB="$WORK/sebas.db" \
SEBAS_STATE_FILE="$WORK/state.json" \
SEBAS_ROUTER_PROVIDER_OVERLAY="$WORK/providers.json" \
SEBAS_WEBUI_AUTH_FILE="$WORK/webui-auth.json" \
SEBAS_PROJECTS_PATH="$WORK/projects.json" \
  "$BIN" core -c "$WORK/config.toml" --router --debug --webui --webui-port "$PORT" \
  > "$WORK/core.log" 2>&1 &
CORE_PID=$!

# --- 就绪轮询（/health ok 才交给 playwright） --------------------------------
for _ in $(seq 1 120); do
  if curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
    echo "[e2e] sandbox ready on port $PORT (dir: $WORK)"
    break
  fi
  if ! kill -0 "$CORE_PID" 2>/dev/null; then
    echo "error: sebas core exited during startup; log:" >&2
    tail -30 "$WORK/core.log" >&2 || true
    exit 1
  fi
  sleep 0.5
done
curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 || {
  echo "error: sandbox not healthy after 60s; log:" >&2
  tail -30 "$WORK/core.log" >&2 || true
  exit 1
}

# 前台持守：playwright 轮询 url 就绪后开始跑用例；结束时杀掉进程组，
# trap 完成清理。wait 被信号打断后进程随 trap 退出。
wait "$CORE_PID" || true
