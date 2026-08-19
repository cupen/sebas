#!/usr/bin/env bash
# E2E: `sebas gateway --debug` sits on [gateway] listen, serves the built-in
# `test` model (self answers, no upstream). Used to verify the watchdog
# --debug spawn path independent of the watchdog.
set -u

SEBAS_BIN="$(cd "$(dirname "$0")/.." && pwd)/target/debug/sebas"
WORK="$(mktemp -d)"
CFG="$WORK/gw.toml"
LOG="$WORK/gw.log"
trap 'pkill -f "sebas gateway --config $CFG" 2>/dev/null; rm -rf "$WORK"' EXIT

# --- config (printf, not heredoc) ---
printf '%s\n' \
  '[gateway]' \
  'listen = "127.0.0.1:8787"' \
  'debug = true' \
  '' \
  '[provider "test"]' \
  'model = "claude"' \
  'base_url = "gateway://self"' \
  > "$CFG"

# --- launch ---
echo "=== launching gateway --debug (log: $LOG) ==="
export RUST_LOG=info
export SEBAS_CONTROL_SECRET='t s'
"$SEBAS_BIN" gateway --config "$CFG" --debug >"$LOG" 2>&1 &
GW_PID=$!

# --- wait for :8787 ---
echo "=== waiting for listen :8787 ==="
for i in $(seq 1 20); do
  if curl -s -o /dev/null http://127.0.0.1:8787/ 2>/dev/null; then break; fi
  sleep 0.2
done

echo "=== gateway log ==="
cat "$LOG"
echo
echo "=== curl /v1/messages (model=test) ==="
RESP=$(curl -s -X POST http://127.0.0.1:8787/v1/messages \
  -H 'content-type: application/json' \
  -d '{"model":"test","messages":[{"role":"user","content":"ping"}]}')
echo "$RESP"
echo
echo "=== stopping gateway (pid $GWPID) ==="
kill "$GWPID" 2>/dev/null
wait "$GWPID" 2>/dev/null
echo "done"
