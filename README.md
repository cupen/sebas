# sebas

A Rust daemon that bridges Claude Code (via ACP) to Feishu. Run Claude Code remotely from any Feishu chat.

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
# 5. ./target/release/sebas -config ./config.toml
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

This is an MVP / work-in-progress. The WebSocket long-connection URL and handshake against a real Feishu workspace have not been verified end-to-end. Coverage tooling (cargo-llvm-cov) is not yet configured in CI. The `record` subcommand (§4.4 of the spec) is deferred.

## Known limitations

- SessionMap is in-memory only and lost on restart; sessions are restored lazily on next message per chat, but any in-progress work in the previous Claude Code session is not resumable from the child's perspective.
- A `tests/bin/fake-claude.rs` development binary exists for integration testing, but no production test harness with real ACP protocol fixtures is in place yet.
- Slash commands `/compact`, `/cost`, `/model`, `/cd` are dispatched to the ACP backend but their protocol-level behavior has not been validated end-to-end.
- No coverage thresholds enforced in CI; overall ≥80%, router ≥90%, cards ≥90% targets are stated goals only.
- The `record` subcommand for fixture capture (§4.4) is not implemented.

## Manual smoke test

1. Start sebas; confirm `sebas started` log line
2. Send "hello" via Feishu DM; expect emoji reaction sequence on the response card
3. Send "list the files here"; expect permission card for Bash — click Allow
4. Send "/new"; confirm new session spawned
5. Send "/sessions"; confirm both visible
6. Restart sebas (`Ctrl-C`, then restart); send message in same chat; confirm session resumes
