#!/usr/bin/env bash
# sebas gateway admin API e2e 验证脚本（Task 7.1 /
# gateway-admin-api-and-model-aliases）。
#
# ⚠️ 已死脚本（STALE / DEAD）——本脚本自 `remove-gateway-residue`（commit
#     ca75717）起就没有任何仓库入口运行它（`tasks.py` 与 CI 均不引用），且它
#    仍指向前更名时代（pre-rename）的命令面与配置形态：
#       - `sebas gateway` 子命令已不存在（现行 `sebas router`，见 src/cli.rs）；
#       - 配置用旧的 `[gateway]` 顶层段与 `[[gateway.keys]]`，现行是
#         `[router]` + `[provider.*]`（`deny_unknown_fields` 会拒绝旧键）；
#       - env `SEBAS_GATEWAY_PROVIDER_OVERLAY` 已更名（且本轮退休）；
#       - 探针 `/healthz` 与 admin provider 写入端点（`POST /admin/providers`
#         已退休，见 sebas-router/src/admin.rs:50）。
#     本脚本未随 retire-legacy-state-json 一并删除（超出该 change 的文件范围），
#     但**不要把它当作可跑的验证入口**——它的整条命令面都已失效。
#
# 全程无真上游、无 secret（loopback 放行）、无外部依赖（python3 mock 上游）：
#   1. cargo build → 生成临时 config（空 providers + overlay 独立临时路径）
#   2. 起 python3 mock Anthropic 上游（/v1/messages 回固定 SSE）
#   3. 起 sebas gateway（loopback，无 SEBAS_CONTROL_SECRET）
#   4. admin 建 provider（指向 mock 上游）+ 模型别名 → 经别名路由发请求命中
#      mock 上游（断言上游确实被打了 mock 的 api_key）
#   5. 抓 /metrics 断言 requests_total 计数 > 0；/admin/stats 非 0
#   6. 清理
#
# 注：原先还有一条「外部改写 providers.json 测热更新」用例（旧第 5 步，见下文
# 删除点注释）——retire-legacy-state-json 3.5 之后 router 不再读任何 provider
# overlay 文件，该用例测的正是被退休的机制，故整段删除。
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
echo "[1/5] cargo build --bin sebas"
( cd "$REPO_ROOT" && cargo build --bin sebas ) || { echo "error: build 失败"; exit 1; }

# ---- 2. config：空 provider 集（provider 全靠 admin API 建） ----
echo "[2/5] 生成临时 config → $CONFIG"
{
  echo "[gateway]"
  echo "listen = \"127.0.0.1:${PORT}\""
  echo "usage_db = \"$TMPDIR/usage.db\""
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
echo "[3/5] 启动 sebas gateway（无 secret，loopback）"
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
echo "[4/5] admin 建 provider + 别名，经别名路由请求 mock 上游"
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

# ---- (原第 5 步) 已删除：外部改写 providers.json 测热更新 ----
#
# 这里原有一条用例：inline python3 往 $OVERLAY（config 的 provider_overlay）
# 里注入 providers["external"]，再轮询 `GET $BASE/admin/providers` 断言
# `"external"` 出现（热生效）。
#
# 该用例整段删除，理由：它验证的正是本 change 退休掉的机制
# （retire-legacy-state-json 3.5 —— router 不再读取任何 provider overlay
# 文件，overlay 读取与文件监视一并删除）。文件既不再被写入，也不再被读取，
# 「外部改写文件触发重载」不再是一条能力，保留该用例只会断言一个已消失的
# 行为。provider 热更新现在的权威来源是 **core state channel**：provider
# 变更经 channel 快照下发，router 订阅后 swap 内核（见
# sebas-router/src/core_channel.rs 与 hot_reload.rs 的种子逻辑）。
# 经 channel 验证热更新的用例由进程级 e2e 套件承担
# （`invoke testsuite-e2e`），不在本脚本内重建。

# ---- 5. /metrics + /admin/stats 计数断言 ----
echo "[5/5] /metrics 与 /admin/stats 断言"
metrics="$(curl -sS --max-time 5 "$BASE/metrics")"
echo "$metrics" | grep -q '# TYPE sebas_gateway_requests_total counter' || {
  echo "error: /metrics 缺 gateway_requests_total"; echo "$metrics" | head -20; exit 1; }
echo "$metrics" | grep -q 'sebas_gateway_requests_total{provider="mock"' || {
  echo "error: /metrics 无 mock provider 计数"; exit 1; }
stats="$(curl -sS --max-time 5 "$BASE/admin/stats")"
# 内核里 provider 数 == 2（config 的 seed + admin API 建的 mock）。此前是 3，
# 因为外部改写用例注入的 external 也算一个；该用例删除后不再有第三个。
# per_provider 只含有流量的 provider（mock），另断言其 requests 计数在场。
echo "$stats" | grep -q '"providers":2' || { echo "error: /admin/stats provider 数不等于 seed + mock"; echo "$stats"; exit 1; }
echo "$stats" | grep -q '"name":"mock"' || { echo "error: /admin/stats 未见 mock"; echo "$stats"; exit 1; }
echo "  /metrics 计数与 /admin/stats 均正常"

echo
echo "PASS: gateway admin e2e 全流程通过"
