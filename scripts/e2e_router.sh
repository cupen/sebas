#!/usr/bin/env bash
# sebas router e2e 验证脚本（Task 11 / sebas-lva.11）。
#
# 流程：cargo build → 生成临时 config → 起 sebas router → curl /healthz →
#   有 ANTHROPIC_API_KEY 时经网关发 POST /v1/messages（stream=true，逐字打印验证 SSE）；
#   有 OPENAI_API_KEY 时发 POST /v1/chat/completions；
#   有 DEEPSEEK_API_KEY 时经网关到 DeepSeek 的 Anthropic 兼容端点发 POST /v1/messages；
#   无 key 则跳过该段并打印 SKIP；
#   末尾校验 usage.jsonl 非空（至少一条 record）；清理进程与临时目录。
#
# 退出码：0 成功（含全 SKIP 路径——这是验证脚本而非 CI 门禁）；非 0 失败。
#
# 用法：./scripts/e2e_gateway.sh [--keep-tmp]
#   --keep-tmp  调试：保留临时目录与 router.log（默认清理）。
#
# 设计要点：
# - 上游 key 仅从 env 读（api_key_env），缺失时该 provider 挂到不可达本地端口
#   （http://127.0.0.1:9）做 smoke，绝不拿假 key 去触达真上游。
# - 始终跑一次 smoke 透传调用（model=smoke-test → 不可达 provider → 502），
#   保证 usage.jsonl 至少落一条 record，使「非空」校验在所有路径下都有意义。
set -euo pipefail

# ---- 路径与常量 ----
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$REPO_ROOT/target/debug/sebas"
GATEWAY_KEY="sk-gw-e2e-${RANDOM}"
TMPDIR="$(mktemp -d -t sebas-router-e2e.XXXXXX)"
CONFIG="$TMPDIR/router.toml"
USAGE_FILE="$TMPDIR/router-usage.jsonl"
LOG_FILE="$TMPDIR/router.log"

# 动态选一个空闲端口，避免与本机已运行的服务碰撞。
PORT="$(python3 -c 'import socket; s=socket.socket()
s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
BASE="http://127.0.0.1:${PORT}"

# 真上游段使用的模型名（可被 env 覆盖）。
ANTHROPIC_MODEL="${ANTHROPIC_MODEL:-claude-3-5-haiku-20241022}"
OPENAI_MODEL="${OPENAI_MODEL:-gpt-4o-mini}"
DEEPSEEK_MODEL="${DEEPSEEK_MODEL:-deepseek-chat}"

# ---- 清理：杀 router 进程 + 删临时目录。trap 覆盖所有退出路径。 ----
GATEWAY_PID=""
cleanup() {
  if [[ -n "$GATEWAY_PID" ]] && kill -0 "$GATEWAY_PID" 2>/dev/null; then
    kill "$GATEWAY_PID" 2>/dev/null || true
    wait "$GATEWAY_PID" 2>/dev/null || true
  fi
  if [[ "$KEEP_TMP" -eq 1 ]]; then
    echo "  (keep-tmp) 临时目录保留于 $TMPDIR"
  else
    rm -rf "$TMPDIR"
  fi
}
trap cleanup EXIT

# ---- 工具检查 ----
for cmd in cargo curl python3; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "error: 需要 $cmd"; exit 1; }
done

# env key 是否就绪（只看是否非空，绝不打印值）。
have() { [[ -n "${1:-}" ]]; }
ANTHROPIC_SET=0; OPENAI_SET=0; DEEPSEEK_SET=0
have "${ANTHROPIC_API_KEY:-}" && ANTHROPIC_SET=1
have "${OPENAI_API_KEY:-}"    && OPENAI_SET=1
have "${DEEPSEEK_API_KEY:-}"  && DEEPSEEK_SET=1

# --keep-tmp：调试用，保留临时目录与 router.log。
KEEP_TMP=0
for arg in "$@"; do
  case "$arg" in
    --keep-tmp) KEEP_TMP=1 ;;
    -h|--help)  sed -n '2,/^$/p' "$0" | sed 's/^# \?//'; exit 0 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

# ---- 1. 构建 ----
echo "[1/7] cargo build --bin sebas"
( cd "$REPO_ROOT" && cargo build --bin sebas ) || { echo "error: build 失败"; exit 1; }

# ---- 2. 生成临时 config ----
# 每个 provider：env key 在 → 真上游（api_key_env）；否则挂不可达 smoke 端口
# （api_key 明文，仅测试用，启动时 warn）。smoke provider 始终存在，供
# smoke 透传调用产出 usage record。
echo "[2/7] 生成临时 config → $CONFIG"
write_provider() { # write_provider <name> <protocol> <real_base> <env_var> <real_model_route>
  local name="$1" proto="$2" real_base="$3" env_var="$4"
  echo "[provider.$name]"
  echo "protocol = \"$proto\""
  if [[ -n "$env_var" && -n "${!env_var:-}" ]]; then
    echo "base_url = \"$real_base\""
    echo "api_key_env = \"$env_var\""
  else
    # 不可达本地端口：仅用于让网关启动 + smoke 透传（绝不触达真上游）。
    echo "base_url = \"http://127.0.0.1:9\""
    echo "api_key = \"sk-e2e-smoke-$name\""
  fi
  echo
}
export GATEWAY_KEY USAGE_FILE PORT
{
  echo "[router]"
  echo "listen = \"127.0.0.1:${PORT}\""
  echo "usage_file = \"$USAGE_FILE\""
  echo
  echo "[[router.keys]]"
  echo "key = \"$GATEWAY_KEY\""
  echo "name = \"e2e\""
  echo "rpm = 600"
  echo "daily_token_quota = 100_000_000"
  echo
  write_provider anthropic anthropic "https://api.anthropic.com"     "ANTHROPIC_API_KEY"
  write_provider openai     openai     "https://api.openai.com/v1"   "OPENAI_API_KEY"
  write_provider deepseek   anthropic  "https://api.deepseek.com/anthropic" "DEEPSEEK_API_KEY"
  # 始终存在的 smoke provider（不可达），供 smoke 透传调用产出 usage record。
  echo "[provider.smoke]"
  echo "protocol = \"anthropic\""
  echo "base_url = \"http://127.0.0.1:9\""
  echo "api_key = \"sk-e2e-smoke-plumbing\""
  echo
  echo "[router.routes]"
  echo "\"claude-*\" = [\"anthropic\"]"
  echo "\"gpt-*\" = [\"openai\"]"
  echo "\"deepseek-*\" = [\"deepseek\"]"
  echo "\"smoke-*\" = [\"smoke\"]"
} > "$CONFIG"

# ---- 3. 起 router ----
echo "[3/7] 启动 sebas router"
"$BIN" router --config "$CONFIG" >"$LOG_FILE" 2>&1 &
GATEWAY_PID=$!
ready=0
for _ in $(seq 1 50); do
  if curl -sS --max-time 1 "$BASE/healthz" >/dev/null 2>&1; then ready=1; break; fi
  kill -0 "$GATEWAY_PID" 2>/dev/null || { echo "error: router 进程已退出"; cat "$LOG_FILE"; exit 1; }
  sleep 0.2
done
if [[ $ready -ne 1 ]]; then
  echo "error: router 在 10s 内未就绪"; cat "$LOG_FILE"; exit 1
fi
echo "  router 就绪 (PID $GATEWAY_PID, listen $BASE)"

# ---- 4. /healthz ----
echo "[4/7] GET /healthz（免鉴权）"
code="$(curl -sS --max-time 5 -o /dev/null -w '%{http_code}' "$BASE/healthz")"
echo "  /healthz → HTTP $code"
[[ "$code" == "200" ]] || { echo "error: /healthz 非 200"; exit 1; }

# ---- 5. smoke 透传调用（始终跑：验证 proxy→usage→jsonl 链路，不依赖真上游） ----
echo "[5/7] smoke 透传调用（model=smoke-test → 不可达 provider，期望 502 并落 usage record）"
smoke_code="$(curl -sS --max-time 15 -o /dev/null -w '%{http_code}' \
  -X POST "$BASE/v1/messages" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -H "x-api-key: $GATEWAY_KEY" \
  -d '{"model":"smoke-test","max_tokens":16,"messages":[{"role":"user","content":"hi"}]}')"
echo "  smoke → HTTP $smoke_code (期望 502)"
[[ "$smoke_code" == "502" ]] || { echo "error: smoke 应返回 502（不可达上游），实际 $smoke_code"; exit 1; }

# ---- 6. 真上游（条件执行） ----
echo "[6/7] 真上游流式验证"
ran_real=0

run_anthropic_sse() {
  local out="$TMPDIR/anthropic.sse"
  echo "  [anthropic] POST /v1/messages stream=true (ANTHROPIC_API_KEY present, model=$ANTHROPIC_MODEL)"
  local s
  s="$(curl -sS --max-time 60 -N -o "$out" -w '%{http_code}' \
    -X POST "$BASE/v1/messages" \
    -H "Content-Type: application/json" \
    -H "anthropic-version: 2023-06-01" \
    -H "x-api-key: $GATEWAY_KEY" \
    -d "{\"model\":\"$ANTHROPIC_MODEL\",\"max_tokens\":16,\"stream\":true,\"messages\":[{\"role\":\"user\",\"content\":\"Reply with just: ok\"}]}")"
  echo "  anthropic → HTTP $s"
  if [[ "$s" == "200" ]] && grep -q '^event: message_start' "$out" && grep -q '^event: content_block_delta' "$out"; then
    echo "  anthropic SSE: message_start + content_block_delta 已确认"
    ran_real=1
  else
    echo "  error: anthropic SSE 校验失败（status=$s）"; head -c 400 "$out"; echo; exit 1
  fi
}

run_openai_sse() {
  local out="$TMPDIR/openai.sse"
  echo "  [openai] POST /v1/chat/completions stream=true (OPENAI_API_KEY present, model=$OPENAI_MODEL)"
  local s
  s="$(curl -sS --max-time 60 -N -o "$out" -w '%{http_code}' \
    -X POST "$BASE/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $GATEWAY_KEY" \
    -d "{\"model\":\"$OPENAI_MODEL\",\"stream\":true,\"messages\":[{\"role\":\"user\",\"content\":\"Reply with just: ok\"}]}")"
  echo "  openai → HTTP $s"
  if [[ "$s" == "200" ]] && grep -q '^data: ' "$out"; then
    echo "  openai SSE: data 行已确认"
    ran_real=1
  else
    echo "  error: openai SSE 校验失败（status=$s）"; head -c 400 "$out"; echo; exit 1
  fi
}

run_deepseek_sse() {
  local out="$TMPDIR/deepseek.sse"
  echo "  [deepseek] POST /v1/messages stream=true via DeepSeek anthropic-compat (DEEPSEEK_API_KEY present, model=$DEEPSEEK_MODEL)"
  local s
  s="$(curl -sS --max-time 60 -N -o "$out" -w '%{http_code}' \
    -X POST "$BASE/v1/messages" \
    -H "Content-Type: application/json" \
    -H "anthropic-version: 2023-06-01" \
    -H "x-api-key: $GATEWAY_KEY" \
    -d "{\"model\":\"$DEEPSEEK_MODEL\",\"max_tokens\":16,\"stream\":true,\"messages\":[{\"role\":\"user\",\"content\":\"Reply with just: ok\"}]}")"
  echo "  deepseek → HTTP $s"
  if [[ "$s" == "200" ]] && grep -q '^event: message_start' "$out" && grep -q '^event: content_block_delta' "$out"; then
    echo "  deepseek SSE: message_start + content_block_delta 已确认"
    ran_real=1
  else
    echo "  error: deepseek SSE 校验失败（status=$s）"; head -c 400 "$out"; echo; exit 1
  fi
}

if [[ $ANTHROPIC_SET -eq 1 ]]; then run_anthropic_sse; else
  echo "  SKIP: anthropic 真上游（ANTHROPIC_API_KEY 未设置）"; fi
if [[ $OPENAI_SET -eq 1 ]];    then run_openai_sse;    else
  echo "  SKIP: openai 真上游（OPENAI_API_KEY 未设置）"; fi
if [[ $DEEPSEEK_SET -eq 1 ]]; then run_deepseek_sse;  else
  echo "  SKIP: deepseek 真上游（DEEPSEEK_API_KEY 未设置）"; fi

# ---- 7. usage.jsonl 非空校验 ----
echo "[7/7] usage.jsonl 非空校验 → $USAGE_FILE"
# writer 是异步 mpsc，轮询直到至少一行出现（最多 3s）。
lines=0
for _ in $(seq 1 30); do
  [[ -f "$USAGE_FILE" ]] && lines=$(grep -c . "$USAGE_FILE" 2>/dev/null || echo 0)
  [[ "$lines" -gt 0 ]] && break
  sleep 0.1
done
if [[ "$lines" -le 0 ]]; then
  echo "error: usage.jsonl 为空（无 record 落盘）"; exit 1
fi
echo "  usage.jsonl: $lines 行 record"
# 抽样校验：每行是合法 JSON，且 smoke 段的 502 record 在场。
python3 - "$USAGE_FILE" <<'PY'
import json, sys
path = sys.argv[1]
bad = 0; has_502 = False; has_tokens = False
with open(path, encoding="utf-8") as f:
    for ln in f:
        ln = ln.strip()
        if not ln: continue
        try:
            r = json.loads(ln)
        except Exception:
            bad += 1; continue
        if r.get("status") == 502: has_502 = True
        if r.get("input_tokens") is not None or r.get("output_tokens") is not None:
            has_tokens = True
if bad:
    print(f"  error: {bad} 行非法 JSON"); sys.exit(1)
print(f"  合法 JSON，全部 record 可解析")
print(f"  smoke 502 record: {'在场' if has_502 else '缺失'}")
print(f"  含 token 计数的 record: {'有' if has_tokens else '无（全 SKIP 路径下正常）'}")
PY

echo
echo "=== e2e 通过 ==="
if [[ $ran_real -eq 1 ]]; then
  echo "  真上游流式验证：已跑通"
else
  echo "  真上游流式验证：全 SKIP（无 env key）——smoke 链路已验证 proxy+usage+jsonl 闭环"
fi
exit 0
