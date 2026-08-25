#!/usr/bin/env bash
# sebas gateway admin API e2e 验证脚本（Task 7.1 /
# gateway-admin-api-and-model-aliases）。
#
# 全程无真上游、无 secret（loopback 放行）、无外部依赖（python3 mock 上游）：
#   1. cargo build → 生成临时 config（空 providers + overlay 独立临时路径）
#   2. 起 python3 mock Anthropic 上游（/v1/messages 回固定 SSE）
#   3. 起 sebas gateway（loopback，无 SEBAS_CONTROL_SECRET）
#   4. admin 建 provider（指向 mock 上游）+ 模型别名 → 经别名路由发请求命中
#      mock 上游（断言上游确实被打了 mock 的 api_key）
#   5. 外部改 providers.json（加第二个 provider）→ 轮询 /admin/providers
#      断言热生效
#   6. 抓 /metrics 断言 requests_total 计数 > 0；/admin/stats 非 0
#   7. 清理
#
# 退出码：0 成功；非 0 失败。
# 用法：./scripts/e2e_gateway_admin.sh [--keep-tmp]
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$REPO_ROOT/target/debug/sebas"
TMPDIR="$(mktemp -d -t sebas-gw-admin-e2e.XXXXXX)"
CONFIG="$TMPDIR/gateway.toml"
OVERLAY="$TMPDIR/providers.json"
LOG_FILE="$TMPDIR/gateway.log"

free_port() {
  python3 -c 'import socket
s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}
PORT="$(free_port)"
UPSTREAM_PORT="$(free_port)"
BASE="http://127.0.0.1:${PORT}"
UPSTREAM="http://127.0.0.1:${UPSTREAM_PORT}"

# ---- mock Anthropic 上游：记录 x-api-key，回固定 SSE ----
UPSTREAM_LOG="$TMPDIR/upstream.log"
python3 - "$UPSTREAM_PORT" "$UPSTREAM_LOG" <<'PY' &
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

port, logpath = int(sys.argv[1]), sys.argv[2]

class H(BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        key = self.headers.get("x-api-key", "")
        with open(logpath, "a", encoding="utf-8") as f:
            f.write(json.dumps({"path": self.path, "api_key": key}) + "\n")
        if self.path == "/v1/messages":
            sse = (
                "event: message_start\n"
                'data: {"type":"message_start","message":{"id":"msg_mock","model":"mock-model"}}\n\n'
                "event: content_block_delta\n"
                'data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"ok"}}\n\n'
                "event: message_stop\n"
                'data: {"type":"message_stop"}\n\n'
            )
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.send_header("content-length", str(len(sse)))
            self.end_headers()
            self.wfile.write(sse.encode())
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, *a):
        pass

HTTPServer(("127.0.0.1", port), H).serve_forever()
PY
UPSTREAM_PID=$!

GATEWAY_PID=""
cleanup() {
  [[ -n "$GATEWAY_PID" ]] && kill "$GATEWAY_PID" 2>/dev/null || true
  [[ -n "$UPSTREAM_PID" ]] && kill "$UPSTREAM_PID" 2>/dev/null || true
  if [[ "${KEEP_TMP:-0}" -eq 1 ]]; then
    echo "  (keep-tmp) 临时目录保留于 $TMPDIR"
  else
    rm -rf "$TMPDIR"
  fi
}
trap cleanup EXIT

KEEP_TMP=0
for arg in "$@"; do
  case "$arg" in
    --keep-tmp) KEEP_TMP=1 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

# ---- 1. build ----
echo "[1/6] cargo build --bin sebas"
( cd "$REPO_ROOT" && cargo build --bin sebas ) || { echo "error: build 失败"; exit 1; }

# ---- 2. config：空 provider 集（provider 全靠 admin API 建） ----
echo "[2/6] 生成临时 config → $CONFIG"
{
  echo "[gateway]"
  echo "listen = \"127.0.0.1:${PORT}\""
  echo "usage_file = \"$TMPDIR/usage.jsonl\""
  echo "provider_overlay = \"$OVERLAY\""
  echo
  echo "[[gateway.keys]]"
  echo "key = \"sk-gw-admin-e2e\""
  echo "name = \"e2e\""
  echo
  # config 校验要求至少一个 provider；放一个不可达占位（admin 建的才是主角）。
  echo "[provider.seed]"
  echo "base_url_anthropic = \"http://127.0.0.1:9\""
  echo "api_key = \"sk-e2e-seed\""
} > "$CONFIG"

# ---- 3. 起 gateway（无 secret，loopback 放行 admin）----
echo "[3/6] 启动 sebas gateway（无 secret，loopback）"
unset SEBAS_CONTROL_SECRET || true
SEBAS_GATEWAY_PROVIDER_OVERLAY="$OVERLAY" \
  SEBAS_GATEWAY_CONFIG="$CONFIG" \
  "$BIN" gateway --config "$CONFIG" >"$LOG_FILE" 2>&1 &
GATEWAY_PID=$!
for _ in $(seq 1 50); do
  curl -sS --max-time 1 "$BASE/healthz" >/dev/null 2>&1 && break
  kill -0 "$GATEWAY_PID" 2>/dev/null || { echo "error: gateway 已退出"; cat "$LOG_FILE"; exit 1; }
  sleep 0.2
done
curl -sS --max-time 1 "$BASE/healthz" >/dev/null 2>&1 || { echo "error: gateway 未就绪"; cat "$LOG_FILE"; exit 1; }
echo "  gateway 就绪 ($BASE)"

# ---- 4. admin 建 provider + 别名 → 经别名请求命中 mock 上游 ----
echo "[4/6] admin 建 provider + 别名，经别名路由请求 mock 上游"
code="$(curl -sS --max-time 5 -o /dev/null -w '%{http_code}' \
  -X POST "$BASE/admin/providers" \
  -H 'content-type: application/json' \
  -d "{\"name\":\"mock\",\"base_url_anthropic\":\"$UPSTREAM\",\"api_key\":\"sk-mock-upstream\"}")"
[[ "$code" == "201" ]] || { echo "error: 建 provider → $code"; cat "$LOG_FILE"; exit 1; }
echo "  provider mock 已建 (HTTP $code)"

code="$(curl -sS --max-time 5 -o /dev/null -w '%{http_code}' \
  -X POST "$BASE/admin/model-aliases" \
  -H 'content-type: application/json' \
  -d '{"alias":"fast","provider":"mock","upstream_model":"mock-model"}')"
[[ "$code" == "201" ]] || { echo "error: 建别名 → $code"; exit 1; }
echo "  alias fast → mock 已建 (HTTP $code)"

out="$TMPDIR/alias.sse"
s="$(curl -sS --max-time 15 -N -o "$out" -w '%{http_code}' \
  -X POST "$BASE/v1/messages" \
  -H "content-type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -H "x-api-key: sk-gw-admin-e2e" \
  -d '{"model":"fast","max_tokens":16,"messages":[{"role":"user","content":"hi"}]}')"
[[ "$s" == "200" ]] || { echo "error: 经别名请求 → $s"; cat "$LOG_FILE"; exit 1; }
grep -q 'event: content_block_delta' "$out" || { echo "error: 别名请求未命中 mock SSE"; exit 1; }
# 上游确实收到请求，且带上了我们设的 api_key。
grep -q '"api_key": "sk-mock-upstream"' "$UPSTREAM_LOG" || {
  echo "error: mock 上游未收到带 api_key 的请求"; cat "$UPSTREAM_LOG"; exit 1; }
echo "  经 alias fast 请求 → mock 上游命中（api_key 已透传）"

# ---- 5. 外部改 providers.json → 热生效 ----
echo "[5/6] 外部改 providers.json，断言热生效"
python3 - "$OVERLAY" <<'PY'
import json, sys
path = sys.argv[1]
with open(path, encoding="utf-8") as f:
    data = json.load(f)
data.setdefault("providers", {})["external"] = {
    "base_url_anthropic": "http://127.0.0.1:9", "api_key": "sk-external"
}
with open(path, "w", encoding="utf-8") as f:
    json.dump(data, f, ensure_ascii=False, indent=2)
PY
hot=1
last_list=""
for _ in $(seq 1 50); do
  last_list="$(curl -sS --max-time 2 "$BASE/admin/providers" || true)"
  if echo "$last_list" | grep -q '"external"'; then hot=0; break; fi
  sleep 0.2
done
[[ $hot -eq 0 ]] || { echo "error: 外部写入 10s 内未热生效；最后 providers 列表: $last_list"; grep -i reload "$LOG_FILE"; exit 1; }
echo "  providers.json 外部写入已热生效（provider external 可见）"

# ---- 6. /metrics + /admin/stats 计数断言 ----
echo "[6/6] /metrics 与 /admin/stats 断言"
metrics="$(curl -sS --max-time 5 "$BASE/metrics")"
echo "$metrics" | grep -q '# TYPE sebas_gateway_requests_total counter' || {
  echo "error: /metrics 缺 gateway_requests_total"; echo "$metrics" | head -20; exit 1; }
echo "$metrics" | grep -q 'sebas_gateway_requests_total{provider="mock"' || {
  echo "error: /metrics 无 mock provider 计数"; exit 1; }
stats="$(curl -sS --max-time 5 "$BASE/admin/stats")"
# 热生效后 kernel 里 provider 数 ≥ 3（seed + mock + external）；per_provider
# 只含有流量的 provider（mock），另断言其 requests 计数在场。
echo "$stats" | grep -q '"providers":3' || { echo "error: /admin/stats provider 数未反映热生效"; echo "$stats"; exit 1; }
echo "$stats" | grep -q '"name":"mock"' || { echo "error: /admin/stats 未见 mock"; echo "$stats"; exit 1; }
echo "  /metrics 计数与 /admin/stats 均正常"

echo
echo "PASS: gateway admin e2e 全流程通过"
