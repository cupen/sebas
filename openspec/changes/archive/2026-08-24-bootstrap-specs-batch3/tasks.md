## 1. feishu-cards

- [x] 1.1 Write `specs/feishu-cards/spec.md` from the code-research inventory (per-turn card model, structure, debounce, terminal rendering, thinking, truncation, budget/rotation, interactive JSON, help card, error cards, lifecycle, theme)
- [x] 1.2 Verify claims against source with file:line citations from the research agent (card_state.rs, session_boot.rs, cards.rs)

## 2. feishu-reactions

- [x] 2.1 Write `specs/feishu-reactions/spec.md` (vocabulary, ack, FSM, swap, target selection, terminals, permission wait, back-pressure, cadence)
- [x] 2.2 Resolve the stale-test conflict: `card_state_test.rs` terminal-reaction assertions are obsolete; the passing e2e suite (`full_e2e_test`, `pump_unit_test`) is authoritative — spec documents current code

## 3. provider-management

- [x] 3.1 Write `specs/provider-management/spec.md` (main card, mode switching, CRUD forms, masking, probing, spawn resolution ×3, --model precedence, error abort, overlay self-heal)
- [x] 3.2 Document probe as single-URL choice (no anthropi fallback) — matches code, diverges from `.claude/rules/how-to.md` prose

## 4. gateway-core

- [x] 4.1 Write `specs/gateway-core/spec.md` (endpoints, sniffing, prefixes, model extraction, buffering, routing order, rename, upstream construction, SSE passthrough, buffered relay, error translation, timeouts, no translation, debug provider)
- [x] 4.2 Drop "body sniffing" framing from research — protocol detection is path+header only; spec reflects that

## 5. gateway-auth-rate-limit

- [x] 5.1 Write `specs/gateway-auth-rate-limit/spec.md` (auth, open gateway, token bucket, usage pipeline + content, SSE tee, JSON parse, access log, ordering)
- [x] 5.2 Omit daily token quota — not implemented (stale comment only); proposal updated accordingly

## 6. Validation

- [x] 6.1 `openspec validate bootstrap-specs-batch3 --strict` passes
- [x] 6.2 Review: no requirement contradicts current code; discrepancies recorded for the final report (probe URL choice, quota absence, stale card_state tests)
