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
