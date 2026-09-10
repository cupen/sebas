## Context

`extract-im-service` moved IM presentation (cards + reactions) from the core process into the detached `sebas im` service. The `feishu-reactions` spec (from 08-24 bootstrap) still describes the pre-refactor pipeline: core emits `Out::React` consumed by an in-core Feishu adapter, targeting the user's input message, with a `⏳` back-pressure reaction and a shared debounced pump. Reality: the core's outbound pump drops chat-facing `Out`; the im frontend derives phase reactions from `SessionInfo.phase` over the session channel and applies them to the card message it owns. The `⏳` back-pressure reaction and the per-message ack are not rendered by the im frontend.

## Goals / Non-Goals

**Goals:**
- Re-home the reaction pipeline to the IM service: phase reactions driven by observed `SessionInfo.phase`, targeting the card message.
- Reconcile terminal-state semantics (no `CrossMark` emitted) and swap/cadence semantics with the im implementation.
- Make the ownership boundary explicit (core SHALL NOT render IM reactions/cards; im SHALL).

**Non-Goals:**
- Do not re-introduce the `⏳` back-pressure reaction in this change (removed; see proposal Non-goals). A future change may add an im-rendered back-pressure signal.
- Do not move card rendering into core.

## Decisions

- **Target = card message**: adopt the deployed behavior (reactions on `card_msg_id`) rather than the old input-message targeting, since the im frontend owns the card and the input-msg-id bookkeeping is dead. Trade-off: reactions are no longer on the user's message; accepted because the card is the stable presentation.
- **Phase source**: the im frontend reacts on observed `SessionInfo.phase` diffs (seed→working→terminal), not on core-emitted `Out::React`. Cadence bounded by the frontend poll/observe interval.
- **Remove** the back-pressure and shared-debounce-pump requirements; document that mid-turn queueing is reflected via card/session content instead of a dedicated reaction.
- Add an explicit `im-service` requirement that the frontend renders session reactions from `SessionInfo.phase` and re-aligns after reconnect/resync.

## Risks / Trade-offs

- Reaction latency now follows the im poll cadence (≈250 ms) rather than a shared pump tick; acceptable for a status emoji.
- Terminal `DONE` reaction relies on the im observing the phase change; if a resync is dropped, the reaction may lag until the next observed phase — mitigated by the snapshot realign requirement.
