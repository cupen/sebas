#!/usr/bin/env bash
# Run the full watchdog --debug upgrade test in a sandboxed data dir.
#
#   watchdog --debug : core `run --config` child (fake Feishu creds, stays
#                      alive via test-build), separate `router --debug` HTTP
#                      child on [router] listen 127.0.0.1:8787, control-plane
#                      Unix socket, per-run secret.
#   test sequence    : dry-run dev -> real dev -> dry-run rollback -> rollback
#   the router --debug child must stay up across every core restart.
set -u

REPO="$(cd "$(dirname "$0")/.." && pwd)"
EXE=/home/cupen/workbench/repos-tool/sebas/target/debug/sebas
WORK=$(mktemp -d /tmp/sebas-wdtest.XXXXXX)
DATA="$WORK/data"            # watchdog data dir (install/current/rollback)
CFG="$WORK/config.toml"
LOG="$WORK/watchdog.log"
SOCK=""                      # discovered after launch via control status
trap 'kill ${WD_PID:-} ${GWPID:-} 2>/dev/null' EXIT

echo "### sandbox: $WORK"
echo "### build debug binary"
(cd "$REPO" && cargo build 2>&1 | tail -3)

cat > "$CFG" <<'CONF'
[feishu]
app_id = "test-app"
app_secret = "test-secret"
owner_id = "ou_test"

[router]
listen = "127.0.0.1:8787"

[watchdog.upgrade]
github_repo = "cupen/sebas"
check_on_start = false
max_retries = 1
retry_delay_secs = 1
updater_timeout_secs = 600
dev_build_timeout_secs = 1800

[watchdog.storage]
data_dir = "/REPLACE_DATA"

[watchdog.webui]
enabled = false
CONF
sed -i "s|/REPLACE|$DATA|" "$CFG"

echo "### launch watchdog --debug"
mkdir -p "$WORK"
SEBAS_IPC=1 \
"$EXE" run --debug --config "$CFG" >"$LOG" 2>&1 &
WATCH_PID=$!
echo "watchdog pid=$WATCH_PID, log=$LOG"

# discover control socket from the watchdog log
for i in $(seq 1 30); do
  SOCK=$(grep -oP 'control RPC listening at \K\S+' "$LOG" 2>/dev/null | head -1)
  [ -n "${SOCK:-}" ] && break
  sleep 0.2
done
echo "control socket=$SOCK"

# discover the per-run secret from the watchdog's own child (/proc)
SECRET=""
for cpid in $(pgrep -P "$WATCH_PID" 2>/dev/null); do
  if tr '\0' '\n' < /proc/$cpid/environ 2>/dev/null | grep -q '^SEBAS_CONTROL_SECRET='; then
    SECRET=$(tr '\0' '\n' < /proc/$cpid/environ 2>/dev/null | sed -n 's/^SEBAS_CONTROL_SECRET=//p' | head -1)
    break
  fi
done
if [ -z "$SECRET" ]; then
  # fall back: the watchdog itself holds it (we set it on our own environment)
  SECRET="${SEBAS_CONTROL_SECRET:-}"
fi
echo "SECRET=${SECRET:+set}"

# The core child we spawn: with SEBAS_IPC=1, `run` initializes IPC and stays
# alive; give it a moment. Confirm the debug router is sharing :8787.
echo "=== confirm debug router :8787 (independent child) ==="
sleep 2
curl -s -o /dev/null -w 'router GET / -> %{http_code}\n' http://127.0.0.1:8787/ 2>&1 || echo "(router not yet up)"

# ---- helper ---- send a control request and print the response
request() {
  local op=$1; shift
  echo ">>> control $op $*"
  SEBAS_CONTROL_SECRET="${SECRET:-$SEBAS_CONTROL_SECRET}" "$EXE" control --socket "$SOCK" "$@" 2>&1 || echo "(exit $? from control)"
}

echo ""
echo "### TEST A: dry-run dev update (expect accepted, no restart)"
request update --dev --dry-run
echo "core pid(s) after dry-run:"; pgrep -P "$WATCH_PID" 2>/dev/null
echo

echo "### TEST B: real dev update (cargo build + install + restart core)"
request update --dev
echo "router after dev-update (must still be up):"
curl -s -o /dev/null -w '  router GET / -> %{http_code}\n' http://127.0.0.1:8787/ 2>&1 || echo "  GATEWAY DOWN"
echo

echo "### TEST C: rollback dry-run (expect accepted, no restart)"
request rollback --dry-run
echo

echo "### TEST D: real rollback"
request rollback
echo "router after rollback:"
curl -s -o /dev/null -w '  router GET / -> %{http_code}\n' http://127.0.0.1:8787/ 2>&1 || echo "  GATE DOWN"
echo

echo "### watchdog log tail:"
tail -20 "$LOG"

echo "### data dir state:"
find "$DATA" -maxdepth 3 -type f 2>/dev/null | sort
echo "### final core pid:" $(pgrep -f 'core --config' 2>/dev/null | tr '\n' ' ')

echo "### cleanup"
kill "$WATCH_PID" 2>/dev/null
wait "$WATCH_PID" 2>/dev/null
rm -rf "$WORK"
echo "done"