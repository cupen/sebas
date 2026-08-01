# sebas ↔ Claude Code ACP Bridge (Design)

**Date:** 2026-08-01
**Status:** Proposed — pending review
**Replaces:** implicit dependency on `@agentclientprotocol/claude-agent-acp` (npm shim)

## Problem

Today sebas cannot drive Claude Code end-to-end because:

1. `acp-claude/` config defaults `path = "claude"` (Claude Code CLI), which does **not** speak ACP — spawning it as an ACP agent times out at the `initialize` handshake.
2. The only documented ACP bridge for Claude Code is the npm package `@agentclientprotocol/claude-agent-acp` (Apache-2.0 / MIT), which requires Node.js ≥ 22 (Node 18 cannot parse its `import … with { type: "json" }` ES syntax). Bringing Node into a Rust daemon is a poor fit and the package is a Node ecosystem artifact, not a Rust-native solution.
3. Anthropic publishes no official Rust Claude Code SDK. The TypeScript and Python "Agent SDKs" wrap the Claude Code CLI as a subprocess (`code.claude.com/docs/en/agent-sdk/overview`: "run the CLI as a subprocess"). The documented host protocol for that subprocess is `claude --print --input-format stream-json --output-format stream-json --verbose --include-partial-messages` (newline-delimited JSON on stdio).

## Goal

Replace the npm-shim-shaped hole with a small **in-tree Rust binary** that:

- Speaks ACP over stdio to sebas (using the same `agent-client-protocol = "2.0"` SDK that sebas already uses — single protocol family).
- Spawns `claude --print --input-format stream-json --output-format stream-json --verbose --include-partial-messages` as a child process and translates its stream-json events into ACP `SessionUpdate` notifications.
- Preserves sebas's host-mediated permission flow (Feishu card per tool use) by combining the stream-json event stream with a Claude Code **PreToolUse hook** that round-trips permission decisions through a unix-domain socket.

**Non-goals (YAGNI):**

- Other ACP agents (Codex / Gemini). If a future need arises, swap or add a second bridge binary; sebas's `acp-claude/` client is unchanged either way.
- `session/load` / `loadSession: true` advertisement on the bridge side. The bridge answers `loadSession: false`; sebas's existing fallback path (`session/new` on `load` failure) already covers restart recovery with a fresh session. Wire-level compatibility with the existing fake-claude harness is preserved because the bridge's downstream behavior matches.
- Subagent text/thinking forwarding (`--forward-subagent-text`). sebas's router only consumes 8 `AcpEvent` variants; subagent text already falls on the floor end-to-end.
- Image / audio / embedded-context blocks. sebas currently drops anything that isn't `ContentBlock::Text` (`acp-claude/src/session.rs:120-125`).
- Custom tool registration. Bash / Read / Edit / Glob / Grep are surfaced by Claude Code's default tool set; sebas only reads `title` and `raw_input` verbatim.

## Architecture

```
                ACP over stdio (JSON-RPC v1)
   ┌─────────────────┐                        ┌──────────────────────┐
   │     sebas       │  ◄──────────────────►   │   claude-acp-bridge  │
   │  (acp-claude/   │  AcpAgent::new(path)    │   (new Rust bin,     │
   │   ACP client)   │                        │    agent-client-     │
   └─────────────────┘                        │    protocol server)  │
                                               └──────────┬───────────┘
                                                          │ stream-json over stdio
                                                          ▼
                                            ┌──────────────────────────┐
                                            │  claude --print          │
                                            │   --input-format stream-json
                                            │   --output-format stream-json
                                            │   --verbose              │
                                            │   --include-partial-messages
                                            │   --session-id <uuid>    │
                                            └──────────────┬───────────┘
                                                           │ invokes PreToolUse hook
                                                           ▼
                                            ┌──────────────────────────┐
                                            │ hooks/pretooluse.sh      │
                                            │  (vendored 30-line sh)   │
                                            └──────────────┬───────────┘
                                                           │ writes request JSON, blocks on response
                                                           ▼
                                            unix socket: $XDG_RUNTIME_DIR/sebras-bridge-<pid>.sock
                                                           ▲
                                            ┌──────────────┴───────────┐
                                            │  bridge: permission      │
                                            │  server task — forwards  │
                                            │  to sebas via ACP        │
                                            │  `session/request_       │
                                            │   permission`            │
                                            └──────────────────────────┘
```

The bridge is one Rust binary that owns three concurrent tasks on a single tokio runtime:

1. **ACP server task** — drives `agent-client-protocol` SDK's `AcpServer`/`Agent` trait, reading JSON-RPC requests from sebas's stdin, writing notifications/responses to stdout.
2. **claude driver task** — owns the `claude --print` child process: writes user-turn messages on the child's stdin as stream-json, reads newline-delimited JSON events from stdout.
3. **Permission server task** — owns the unix socket that the PreToolUse hook script talks to; brokers `session/request_permission` requests between claude and sebas.

The two I/O halves communicate over tokio mpsc channels. The bridge does **not** introduce a new inter-process protocol layer — it is a single address space that happens to be spawned as a subprocess for stdio isolation.

## Components (file layout)

New workspace member:

```
sebas/
├── Cargo.toml                          # add "acp-claude-bridge" to [workspace] members
├── acp-claude-bridge/
│   ├── Cargo.toml                      # depends on agent-client-protocol = "2.0", tokio
│   ├── src/
│   │   ├── main.rs                     # entry: wire up 3 tasks, hand off to tokio::main
│   │   ├── server.rs                   # ACP server impl: AcpServer + Agent trait
│   │   ├── claude.rs                   # subprocess management + stream-json framing
│   │   ├── translator.rs               # stream-json event → ACP SessionUpdate mapping
│   │   ├── permission.rs               # unix socket server + bridge to ACP requests
│   │   └── hook.rs                     # writes the PreToolUse hook script + path file
│   └── tests/
│       ├── fake_claude.rs              # speaks stream-json; honors --scenario flag
│       └── bridge_e2e.rs               # spawn bridge + fake claude, drive ACP handshake
├── hooks/
│   └── pretooluse.sh                   # ~30 lines; vendored; chmod +x in build.rs
└── docs/superpowers/specs/             # this file
```

No changes to `acp-claude/`, `router/`, `feishu/`, or `src/` (`run.rs` etc.).

## Wire details

### 1. ACP server (sebas ↔ bridge)

The bridge implements `agent_client_protocol::Agent` (the SDK's server trait) and runs it on stdio. On `initialize` it returns:

```json
{
  "protocolVersion": 1,
  "agentCapabilities": {
    "loadSession": false,
    "promptCapabilities": { "image": false, "audio": false, "embeddedContext": false },
    "mcpCapabilities": { "http": false, "sse": false }
  },
  "agentInfo": { "name": "claude-acp-bridge", "title": "Claude Code ACP Bridge", "version": env!("CARGO_PKG_VERSION") }
}
```

`loadSession: false` is intentional — it triggers sebas's existing `Out::SpawnResume → fallback to SpawnAcp` path (`router/src/router.rs:341-372`), which is what the spec wants when the persisted id is not necessarily valid. **All eight `AcpEvent` variants are still emitted** because the bridge spawns a fresh `claude --print --session-id <uuid>` per `session/new`, and sebas's router maps the uuid as the new session_id — restart recovery continues to work because state is keyed by sebas's session_id, not by Claude Code's.

### 2. claude driver (bridge ↔ claude)

The bridge spawns:

```
claude --print \
  --input-format stream-json \
  --output-format stream-json \
  --verbose \
  --include-partial-messages \
  --session-id <uuid> \
  --mcp-config ""  \
  [--permission-mode default]
```

`--permission-mode default` is required so Claude Code invokes the PreToolUse hook for each tool use instead of auto-deciding. The bridge passes `--session-id` so each `session/new` from sebas maps to a stable Claude Code conversation that can be `--resume`'d by future bridge restarts (out of scope for v1, but the hook is in place).

User-turn messages from `session/prompt` are written as stream-json to the child's stdin:

```json
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"…"}]}}
```

The bridge reads newline-delimited JSON from the child's stdout and dispatches into `translator.rs`.

### 3. Translator (stream-json → ACP)

Only events the bridge actually emits:

| stream-json event | ACP emission |
|---|---|
| `system` (subtype `init`) | `initialized` notification; advertise capabilities |
| `stream_event` with `event.content_block_delta.text_delta` | `SessionUpdate::AgentMessageChunk { content: Text { text } }` |
| `stream_event` with `event.content_block_start` of `tool_use` | `SessionUpdate::ToolCall { title, raw_input }` (deferred — see Permission flow) |
| `stream_event` with `event.content_block_stop` + `event.message_stop` | turn boundary marker; not emitted as ACP event |
| `assistant` (only when subagent / non-stream) | forwarded as `AgentMessageChunk` / `ToolCall` |
| `user` (tool_result blocks) | `SessionUpdate::ToolCallUpdate { status: Completed/Failed, raw_output }` |
| `result` (final line of each turn) | resolve the in-flight `session/prompt` request with `StopReason` |

All other stream-json events are ignored (`api_retry`, `plugin_install`, `hook_*`, `prompt_suggestion`, etc.).

### 4. Permission flow

The bridge registers a Claude Code `PreToolUse` hook that runs `hooks/pretooluse.sh` before each tool use. The script:

1. Reads the tool name and input JSON from stdin (Claude Code's documented format).
2. Writes a request JSON to the unix socket `$XDG_RUNTIME_DIR/sebras-bridge-<pid>.sock` (path is written to a sidecar file by the bridge at startup).
3. Blocks reading the response (timeout = `permission_timeout_secs`, default 0 = unlimited, matching spec §4.1 "永不超时").
4. Exits with code 0 + `{"decision":"approve","reason":""}` to allow, or exits with code 2 + reason on stderr to deny.

The bridge's permission server task:

1. Receives the hook request, extracts `tool_name` + `args`.
2. Sends a `session/request_permission` JSON-RPC request to sebas (the standard ACP pattern; the SDK's request_id ↔ sebas's routing session_id mapping is already done correctly in `acp-claude/src/manager.rs:261-281`).
3. Awaits the `PermissionReply{decision}` from sebas's Feishu card UI.
4. Translates to the hook's expected response (Allow once → `approve`; Deny → `deny` + reason; Allow session → `approve` with a follow-up `settings` patch is out of scope; we answer `approve` only).
5. Writes the response JSON to the socket; the hook script unblocks and exits.

Failure modes:

- Bridge dies mid-permission → script's `read` returns EOF → script exits 2 → Claude Code denies the tool and emits a denial `tool_result`. sebas sees a normal `ToolCallUpdate` with the denial reason.
- Hook script times out (configurable, default 0) → script exits 2 → denial, same as above.
- Socket file stale from a prior crash → bridge uses `socket2` with `SO_REUSEADDR`-equivalent (Linux: unlink-on-bind) and writes pid to the sidecar file so an old script can detect mismatch and exit 2.

### 5. Configuration

`config/config.toml` change — one line in `[acp.claude]`:

```toml
[acp.claude]
# Was: path = "claude"
# Now: path to the bridge binary built from this workspace.
path = "./target/debug/claude-acp-bridge"  # dev
# path = "/usr/local/bin/claude-acp-bridge"  # installed
args = []
work_dir = "<unchanged>"
```

No new top-level config keys. The bridge's own knobs (hook socket dir, permission timeout) become CLI flags with defaults; out-of-scope to expose in sebas's config for v1.

## Testing strategy

TDD, three layers:

1. **Unit tests** in `translator.rs`: feed fixture stream-json lines from `tests/fixtures/acp/stream-json/*.jsonl`, assert the resulting `Vec<SessionUpdate>`. Existing `tests/fixtures/acp/*.jsonl` are consumer-side `AcpEvent` JSON — we add new wire-side fixtures.

2. **Integration test** `tests/bridge_e2e.rs`: spawn the bridge, drive a full ACP handshake + `session/new` + `session/prompt` against `fake_claude.rs` (a new test binary that speaks stream-json instead of ACP — same pattern as existing `tests/bin/fake-claude.rs` but at the lower layer). Assert text deltas, tool calls, and tool results round-trip into the expected `SessionUpdate`s.

3. **Permission round-trip test** `tests/permission_roundtrip.rs`: fake claude triggers a `tool_use` block → bridge synthesizes `session/request_permission` → fake permission responder approves → fake claude receives approval → emits tool result. Pattern mirrors existing `acp-claude/tests/permission_roundtrip.rs`.

4. **E2E with sebas**: an opt-in test (env `SEBAS_TEST_BRIDGE=1`) runs sebas end-to-end with the bridge and a fake claude. Asserts the 8 `AcpEvent` variants still fire in the same FSM transitions documented in `router/src/router.rs:491-511`.

## Migration plan

Three commits, each independently shippable:

1. **`feat(acp-claude-bridge): scaffold new workspace member + stream-json translator`** — adds the crate with `translator.rs` + unit tests. No protocol-level changes. No config change. Existing `path = "claude"` still doesn't work; that's expected.
2. **`feat(acp-claude-bridge): ACP server + claude driver + permission broker`** — implements the full bridge binary. `config.toml.example` updates `path` to point at the bridge binary. Existing fake-claude tests still pass (the SDK side never changed). The bridge is wired in but the npm-shim path still works for users who don't rebuild.
3. **`feat(acp-claude-bridge): e2e + permission tests + docs`** — adds the e2e and permission round-trip tests; updates `README.md` "Known limitations" to remove the npm-shim mention; closes the `sebas-vw5.4` Beads issue.

After commit 3, the npm shim is no longer required to run sebas. We do **not** delete the npm-shim-config path from `acp-claude/manager.rs` (it's `cfg.acp.claude.path` — opaque string), so users who want to keep using the shim can still set `path = "npx"` etc.

## Risks and open questions

- **Wire-level ACP compatibility between sebas's `agent-client-protocol` 2.0 SDK and the same SDK in a separate crate.** Both pin to v2.0 of the schema, so this should hold. We will validate via a one-off smoke test before commit 2 lands (spawn the bridge, hand-write an `initialize` JSON-RPC on stdin, assert the response).
- **PreToolUse hook behavior in sandboxed environments.** gVisor / minimal containers may block `unix` sockets or restrict `~/.claude/settings.json` PreToolUse registration. We document the requirement in `README.md`. Not a blocker for local-dev deployments.
- **`--session-id` interaction with `--resume`.** Out of scope for v1. If a future bridge restart needs to honor sebas's persisted session_id, the bridge will need to translate sebas's session_id ↔ claude's session_id — not in this spec.
- **Permission timeout.** Spec §4.1 says "永不超时". We honor that with default 0, but expose `--permission-timeout-secs <n>` for ops use.

## What this spec does NOT change

- `AcpEvent` enum (8 variants, in `acp-claude/src/session.rs:72-114`).
- sebas's router FSM, card renderers, or permission-card button plumbing.
- `config/config.toml`'s `[feishu]`, `[router]`, `[media]`, `[logging]` sections.
- The fake-claude test harness in `tests/bin/fake-claude.rs` (it tests the ACP client side; the bridge replaces the production agent but tests on the client side remain valid).