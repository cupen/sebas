# sebas

A Rust daemon that bridges Claude Code to Feishu (via claude's native stream-json + control protocol). Run Claude Code remotely from any Feishu chat.

## Quick start

```bash
# 1. Create a Feishu app at https://open.feishu.cn/ and grant:
#    - im:message (receive + send)
#    - im:message.group_at_msg (for group messages)
#    - im:message.p2p_msg (for direct messages)
#    Enable "Long connection" event subscription in app capabilities.
# 2. cp config/config.toml.example config.toml
# 3. Edit config.toml: set app_id, app_secret, owner_id (your open_id)
# 4. cargo build --release
# 5. ./target/release/sebas run --config ./config.toml
```

## Configuration

Only 3 fields are required: `feishu.app_id`, `feishu.app_secret`, `feishu.owner_id`. Everything else has defaults — see `docs/superpowers/specs/2026-07-26-sebas-design.md` §6.

## Commands

- `/new` — start fresh session
- `/sessions` — list active sessions
- `/switch <n>` — switch current chat to session n
- `/compact`, `/cost`, `/model`, `/cd`, `/cancel`, `/status` — see `/help`

## Architecture

See `docs/superpowers/specs/2026-07-26-sebas-design.md`.

## Gateway（LLM provider router）

sebas 自带一个双协议面（Anthropic / OpenAI）纯透传 LLM 网关：按模型名把请求路由到对应上游 provider，同协议字节无损转发，附带 per-key 鉴权、限流/配额、用量统计。一句话用途：让 Claude Code（或任意 anthropic/openai SDK）经 `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` 指向本网关，即可把流量分流到国产/海外任一兼容 provider，无需协议转换。

最小配置（见 `config/config.toml.example` 的 `[gateway]` 段）：

```toml
[gateway]
listen = "127.0.0.1:8787"

auth_token = "sk-gw-local-dev"        # 下游客户端 token（鉴权用；单个字符串或数组；
                                      # 不配置 = 不校验 token，裸奔 + 启动 warn）

[provider.anthropic]        # 上游 provider；密钥只从 env 读
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"

[gateway.routes]                      # model（可含 glob）→ provider 数组，
"claude-*" = ["anthropic"]            # 数组顺序 = 优先级（先 = 主）
```

启动：

```bash
./target/debug/sebas gateway --config ./config.toml
# /healthz 免鉴权；其余端点需下游 key（Authorization: Bearer 或 x-api-key）
```

把客户端 BASE_URL 指向网关：

```bash
# Claude Code → 经网关到 Anthropic（或任一 anthropic 协议 provider，如 DeepSeek/Kimi/GLM/MiniMax）
ANTHROPIC_BASE_URL=http://127.0.0.1:8787 ANTHROPIC_API_KEY=sk-gw-local-dev claude

# OpenAI SDK → 经网关到 OpenAI（或任一 openai 兼容 provider）
OPENAI_BASE_URL=http://127.0.0.1:8787 OPENAI_API_KEY=sk-gw-local-dev ...
```

用量记录：每个 proxied 请求落一条 record 到 `gateway.usage_file`（默认 `~/.sebas/gateway-usage.jsonl`），含 ts/key/protocol/model/provider/status/latency/ttft/input+output+cache tokens/error。

端到端验证脚本：`./scripts/e2e_gateway.sh`（build → 起 gateway → `/healthz` → 真 upstream 流式调用 [env key 在场时] → usage.jsonl 非空校验 → 清理）。详见 `docs/superpowers/specs/2026-08-06-gateway-design.md`。

## Status

This is an MVP / work-in-progress. The WebSocket long-connection is fully wired (handshake, event dispatch, exponential-backoff reconnect). No CI is configured at all (tracked: sebas-nya). The `record` subcommand (§4.4 of the spec) is not implemented; `--dump-inbound` plus the `replay` subcommand cover the capture/replay path in the meantime (tracked: sebas-nya).

## Known limitations

- SessionMap is persisted to `state_file` and restored on restart; the first message in a restored chat lazily respawns via claude-native `resume`, so real conversation history carries over. If claude's session files were cleaned in the meantime the resume is rejected and sebas transparently falls back to a fresh session (with a "已开启新会话" notice card). A turn that was mid-flight at shutdown is still lost.
- `tests/bin/fake-claude.rs` is a real-protocol stream-json/control harness used by several integration tests; what is missing is wire-level fixtures (real frames captured from claude) and the canned-binary replay harness (tracked: sebas-vw5.3).
- `/compact` and `/cost` are forwarded to claude as literal prompts; `/model`, `/cd`, `/status`, and `/help` are parsed but not wired (they fall through to HelpText or a no-op). Protocol-level behavior of the forwarded commands has not been validated end-to-end (tracked: sebas-3ti for the unwired commands; tracked: sebas-vw5.4 for end-to-end validation).
- No CI at all; the router ≥90%, cards ≥90%, overall ≥80% coverage targets are stated goals only (tracked: sebas-nya).
- The `record` subcommand for fixture capture (§4.4) is not implemented; `--dump-inbound` + `replay` cover the WS capture/replay path in the meantime (tracked: sebas-nya).

## Manual smoke test

1. Start sebas; confirm `sebas started` log line
2. Send "hello" via Feishu DM; expect emoji reaction sequence on the response card
3. Send "list the files here"; expect permission card for Bash — click Allow
4. Send "/new"; confirm new session spawned
5. Send "/sessions"; confirm both visible
6. Restart sebas (`Ctrl-C`, then restart); send message in same chat; confirm session resumes
