#!/usr/bin/env bash
# Claude Code PreToolUse hook for sebas ↔ bridge permission mediation.
#
# Claude Code invokes this script with the tool name + input on stdin as JSON.
# We write the request to the bridge's unix socket, block reading the
# response, and exit 0 + JSON {"decision":"approve"} on allow, or exit 2
# on deny.
#
# Socket path is read from a sidecar file written by the bridge at startup.

set -euo pipefail

# Read hook input from stdin
input="$(cat)"

# Find sidecar file (bridge writes it next to its socket)
sock_dir="${XDG_RUNTIME_DIR:-/tmp}"
sidecar="$sock_dir/sebras-bridge.sock.path"
if [[ ! -f "$sidecar" ]]; then
  echo "bridge sidecar $sidecar not found" >&2
  exit 2
fi
sock_path="$(cat "$sidecar")"

# Send request, read response
resp_file="$(mktemp)"
trap 'rm -f "$resp_file"' EXIT
if ! printf '%s' "$input" | nc -U -w 600 "$sock_path" > "$resp_file" 2>/dev/null; then
  echo "bridge socket unreachable" >&2
  exit 2
fi
resp="$(cat "$resp_file")"

# Decide
case "$resp" in
  approve)
    printf '{"decision":"approve","reason":""}\n'
    exit 0
    ;;
  deny|*)
    echo "denied by sebas" >&2
    exit 2
    ;;
esac
