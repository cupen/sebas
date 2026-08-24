## 1. Research

- [x] 1.1 Dispatch four parallel research agents (feishu-bridge / session-lifecycle / router-commands / session-persistence), each returning a behavior inventory with file:line citations and verifying-test names
- [x] 1.2 Spot-check surprising findings against source: @bot gate covers p2p (ws_loop.rs:171-187), `Out::HelpText` is a dispatch no-op (dispatch.rs:448-450), `/switch`//`/resume`//`/cd` have no routing arm (inbound.rs:255-257), inbound media passes file keys only (router/mod.rs:826-834)

## 2. Specs

- [x] 2.1 Write `specs/feishu-bridge/spec.md` — WS lifecycle + fatal errors, event dedup, chat-type filter, mention gating, thread targeting, topic-invalid handling, outbound retry, media passthrough
- [x] 2.2 Write `specs/session-lifecycle/spec.md` — session identity, lazy spawn, race protection, dormant resume, terminal teardown, expected-exit, back-pressure, capacity, restart recovery
- [x] 2.3 Write `specs/router-commands/spec.md` — parsing rules, argument validation, session-forwarded commands, /new, /btw, watchdog control commands, /settings, local commands, passthrough + unrouted commands, ack reaction
- [x] 2.4 Write `specs/session-persistence/spec.md` — file layout, v2 schema, v1 migration, overlay reconciliation, legacy field upgrade, corruption tolerance, atomic writes, mode repair, default selection, non-persisted state

## 3. Validation & Archive

- [x] 3.1 Run `openspec validate bootstrap-specs-batch2 --strict` and confirm exit 0
- [x] 3.2 Report discrepancies found during research (p2p missing from default chat-type allowlist; mention gate covers p2p contradicting comments; /switch//resume//cd advertised but unrouted; HelpText fallback silent) to the user — collected for final report
- [x] 3.3 On approval, run `openspec archive bootstrap-specs-batch2 --yes` and confirm four new directories exist under `openspec/specs/` — user pre-approved via goal (finish all remaining specs); discrepancies documented in specs as current behavior
