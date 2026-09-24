#!/usr/bin/env bash
# retire-legacy-state-json 6.3 + acceptance provider_governance 复核脚本。
#
# 在一次性沙箱里：
#   1. 预置三个**遗留文件**（state.json / providers.json / settings.json，
#      各带毒值）——必须不被读取、也不被改动；
#   2. 起 core（含 webui）+ standalone router 子进程；
#   3. 经 webui BFF 把 provider 与 model alias 写进**状态库**；
#   4. 断言 router 经 core 通道热生效：alias 路由命中本地 stub 上游；
#   5. 断言三个遗留文件逐字节未变、库里无 POISON 值。
set -uo pipefail

# 仓库根从脚本自身位置推导（脚本在 <repo>/scripts/ 下）——此前硬编码了某个
# worktree 的绝对路径，换 checkout 就跑不起来。
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SB="$(mktemp -d /tmp/sebas-legacy-63-XXXXXX)"
CORE_LOG="$SB/core.log"
ROUTER_LOG="$SB/router.log"
CORE_PID=""
ROUTER_PID=""

cleanup() {
  [[ -n "$CORE_PID" ]] && kill "$CORE_PID" 2>/dev/null
  [[ -n "$ROUTER_PID" ]] && kill "$ROUTER_PID" 2>/dev/null
  sleep 1
  [[ -n "$CORE_PID" ]] && kill -9 "$CORE_PID" 2>/dev/null
  [[ -n "$ROUTER_PID" ]] && kill -9 "$ROUTER_PID" 2>/dev/null
  rm -rf "$SB"
}
trap cleanup EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
ok()   { echo "ok: $*"; }

echo "sandbox: $SB"
mkdir -p "$SB/work" "$SB/sessions" "$SB/downloads" "$SB/skills" "$SB/xdg-run"

# ---- 1) 预置三个遗留文件（毒值） ----
cat > "$SB/state.json" <<'JSON'
{"version":2,"providers":{"POISON_STATE":{"base_url_anthropic":"https://poison.invalid","api_key":"sk-POISON-STATE"}},"deleted":[],"mode":{"kind":"direct","provider":"POISON_STATE"},"default_selection":{"provider":"POISON_STATE"}}
JSON
cat > "$SB/providers.json" <<'JSON'
{"providers":{"POISON_OVERLAY":{"base_url_anthropic":"https://poison.invalid","api_key":"sk-POISON-OVERLAY"}},"deleted":["POISON_OVERLAY"]}
JSON
cat > "$SB/settings.json" <<'JSON'
{"theme_color":"POISON_COLOR","max_user_text_chars":1,"max_tool_output_chars":2,"fold_long_output":true,"thinking":"hide"}
JSON

b_state=$(sha256sum "$SB/state.json" | cut -d' ' -f1)
b_prov=$(sha256sum "$SB/providers.json" | cut -d' ' -f1)
b_set=$(sha256sum "$SB/settings.json" | cut -d' ' -f1)

cat > "$SB/config.toml" <<TOML
[feishu]
enabled = false

[acp]
default = "claude"
[acp.agents.claude]
driver = "claude"
path = "$REPO/target/debug/fake-claude"
sessions_dir = "$SB/sessions"
work_dir = "$SB/work"

[media]
download_dir = "$SB/downloads"

[workspace]
root = "$SB"

[skills]
dir = "$SB/skills"

[service.core]
channel_path = "$SB/core-channel.sock"

[service.webui]
enabled = true
host = "127.0.0.1"
port = 19771
auth = false

[provider.anthropic]
api_key = "sk-sandbox-dummy"

[router]
listen = "127.0.0.1:18771"
# 不写 `provider_overlay`：该键已退休（retire-legacy-state-json 3.5），写了只会
# 触发废弃告警。本脚本正是要证明 $SB/providers.json 这个遗留文件**不被读取**，
# 所以更不能把它接回配置——它只作为「毒值不被导入」的诱饵存在。

[dispatch]
TOML
# 去掉空的 [dispatch] 段（会被未知键/空表判拒）
sed -i '/^\[dispatch\]$/d' "$SB/config.toml"

[[ -x "$REPO/target/debug/sebas" ]] || fail "target/debug/sebas missing"

# ---- 本地 stub 上游（python 一次应答，记录 model） ----
cat > "$SB/stub.py" <<'PY'
import json, sys, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
asked = []
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get('content-length', 0))
        body = json.loads(self.rfile.read(n) or b'{}')
        asked.append(body.get('model'))
        open(sys.argv[2], 'w').write(json.dumps(asked))
        out = json.dumps({"id":"msg_stub","type":"message","role":"assistant",
                          "model":body.get('model'),"content":[{"type":"text","text":"stub-ok"}],
                          "stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}).encode()
        self.send_response(200); self.send_header('content-type','application/json')
        self.send_header('content-length', str(len(out))); self.end_headers(); self.wfile.write(out)
    def log_message(self, *a): pass
HTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()
PY
STUB_PORT=18772
python3 "$SB/stub.py" "$STUB_PORT" "$SB/asked.json" &
STUB_PID=$!
trap 'kill $STUB_PID 2>/dev/null; cleanup' EXIT

# ---- 2) 起 core（含 webui）与 router 子进程 ----
SEBAS_STATE_DIR="$SB" XDG_RUNTIME_DIR="$SB/xdg-run" \
  "$REPO/target/debug/sebas" run -c "$SB/config.toml" --debug \
  > "$CORE_LOG" 2>&1 &
CORE_PID=$!

for _ in $(seq 1 80); do
  curl -fsS "http://127.0.0.1:19771/health" >/dev/null 2>&1 && break
  sleep 0.5
done
curl -fsS "http://127.0.0.1:19771/health" >/dev/null 2>&1 || {
  echo "--- core.log ---"; tail -30 "$CORE_LOG" >&2; fail "core webui never healthy"; }
ok "core webui healthy"

for _ in $(seq 1 80); do
  curl -sS -o /dev/null "http://127.0.0.1:18771/admin/stats" 2>/dev/null && break
  sleep 0.5
done
ok "router listening"

# ---- 3) 经 webui BFF 写 provider + alias（落状态库） ----
code=$(curl -sS -o "$SB/p.json" -w '%{http_code}' -X POST "http://127.0.0.1:19771/api/providers" \
  -H 'content-type: application/json' \
  -d "{\"name\":\"stub\",\"protocol\":\"anthropic\",\"base_url_anthropic\":\"http://127.0.0.1:$STUB_PORT\",\"api_key\":\"sk-stub-63\"}")
echo "POST /api/providers -> $code $(cat "$SB/p.json")"
[[ "$code" == "201" || "$code" == "200" ]] || fail "provider create failed"

code=$(curl -sS -o "$SB/a.json" -w '%{http_code}' -X POST "http://127.0.0.1:19771/api/model-aliases" \
  -H 'content-type: application/json' \
  -d '{"alias":"my-claude","provider":"stub","upstream_model":"stub-model"}')
echo "POST /api/model-aliases -> $code $(cat "$SB/a.json")"
[[ "$code" == "201" || "$code" == "200" ]] || fail "alias create failed"

# ---- 4) 断言 alias 经 core 通道热生效（不重启 router） ----
hit=0
for i in $(seq 1 60); do
  resp=$(curl -sS -X POST "http://127.0.0.1:18771/v1/messages" \
    -H 'content-type: application/json' \
    -d '{"model":"my-claude","max_tokens":16,"messages":[{"role":"user","content":"hi"}]}' 2>/dev/null)
  if echo "$resp" | grep -q '"msg_stub"'; then hit=1; echo "attempt $i: routed -> $resp"; break; fi
  sleep 1
done
[[ "$hit" == "1" ]] || { echo "last resp: $resp" >&2; fail "alias never routed (channel hot-reload)"; }
ok "alias 经 core 通道热生效"

echo "stub saw models: $(cat "$SB/asked.json" 2>/dev/null)"
grep -q '"stub-model"' "$SB/asked.json" || fail "upstream must receive the aliased model id"
ok "上游收到别名映射后的 model（stub-model）"

# ---- 5) 断言遗留文件逐字节未变 + 库无 POISON ----
for f in state.json providers.json settings.json; do
  [[ -f "$SB/$f" ]] || fail "$f disappeared"
done
[[ "$b_state" == "$(sha256sum "$SB/state.json" | cut -d' ' -f1)" ]] || fail "state.json changed"
[[ "$b_prov"  == "$(sha256sum "$SB/providers.json" | cut -d' ' -f1)" ]] || fail "providers.json changed"
[[ "$b_set"   == "$(sha256sum "$SB/settings.json" | cut -d' ' -f1)" ]] || fail "settings.json changed"
ok "三个遗留文件逐字节未变"

if grep -qa "POISON" "$SB/settings.db" 2>/dev/null || grep -qa "POISON" "$SB/projects.db" 2>/dev/null; then
  fail "状态库里出现 POISON（发生了导入）"
fi
ok "状态库中无 POISON（未导入）"

echo
echo "PASS: 6.3 遗留文件既不被读取、也不被改动；provider/alias 走状态库权威"