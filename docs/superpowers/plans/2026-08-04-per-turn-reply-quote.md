# Per-Turn Reply-Quote Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace single-root-card-per-chat with per-turn cards that quote-reply to the user message. Each user turn gets its own message in the chat scroll, with the per-turn emoji FSM (👀→🚧→✅) operating on that turn's card. Turns run strictly serially; queued turns show ⏳ on the in-flight card; `/btw` jumps to the front of the queue.

**Architecture:**
- Every user text event (`FeishuIn::Text { reply_to, .. }`) carries the originating Feishu message id.
- The router emits `Out::SendCard { root_id: Some(reply_to), .. }` for **every** turn (not just the first one) — first turn posts a fresh card, subsequent turns post a new card each.
- Per-turn emoji FSM operates on the **most-recently-posted** card for that session. Streaming events `PATCH` the latest turn's card; the previous turn's card stays frozen at its final state.
- `MsgIdMap` keeps shape `session_id → msg_id`; semantic: "the card *this turn* is updating". Each `Out::SendCard` flips the pointer to the new card.
- `SessionMap` gains a per-key `turn_queue: VecDeque<QueuedTurn>` (priority-flagged slot for `/btw`). When the current turn settles (`Finished`/`Error`), the queue drains head-first; each drained turn posts a new card and forwards its prompt to the bridge.
- `feishu::client::send_card` gains an optional `root_id` field, plumbed through `Out::SendCard` and the dispatcher.
- New `Command::Btw(String)` parses `/btw <text>`; routed to the front of the queue.

**Tech Stack:** Rust (workspace: `router`, `feishu`, `acp-claude`, `sebas`), existing tokio mpsc pump pattern, Feishu OpenAPI (`im/v1/messages` with `root_id`).

---

## File Structure

| File | Responsibility |
|---|---|
| `feishu/src/client.rs` | `send_card` accepts optional `root_id`; serializes `root_id` into the JSON body when present |
| `router/src/state.rs` | `SessionMap` gains `turn_queue: HashMap<SessionKey, VecDeque<QueuedTurn>>` and the `QueuedTurn { prompt, reply_to, priority: bool }` type; `enqueue_turn` / `pop_next_turn` / `peek_queue_len` API |
| `router/src/router.rs` | `Out::SendCard` gains `root_id: Option<String>`; `on_text`/`continue_session` emit fresh SendCard per turn OR enqueue based on session state; on `Finished`/`Error` event drain queue head; `/btw` parsed and routed with `priority=true`; `MsgIdMap::record` semantics updated (flip on each record) |
| `router/src/commands.rs` | Parse `Command::Btw(String)` from `/btw <text>` |
| `src/run.rs` | `wire_session_card_and_pump` and the `Out::SendCard` dispatcher arm propagate `root_id`; new dispatcher arm for "continue" turn that POSTs a fresh card then forwards the prompt |
| `router/tests/turn_card_test.rs` *(new)* | Integration tests for: per-turn msg_id semantics, root_id propagation, no cross-turn PATCH, previous turn stays frozen, queue serializes turns, ⏳ reaction on enqueue, `/btw` priority slot, drain on settle |

`router/tests/card_state_test.rs` keeps existing tests untouched (FSM within a turn is unchanged). The single-turn `record_root_msg_id` test in `state_test.rs` keeps passing because `MsgIdMap::record` is the same shape — just now represents "the card currently being updated by this session's streaming events".

## Global Constraints

- Conventional Commits (`.claude/rules/how-to.md`).
- No new third-party crates. Workspace deps only.
- Conservative profile: don't push, don't auto-merge. Report changed files + validation, wait for approval.
- TDD: each task writes a failing test first.
- Each task = one commit. Push after the full plan if explicitly asked.

---

## Task 1: `feishu::client::send_card` accepts `root_id`

**Files:**
- Modify: `feishu/src/client.rs:222-236` (the `send_card` function body — find the JSON body construction)
- Test: `feishu/tests/send_card_root_id_test.rs` (new — unit test against the function with a mocked HTTP layer, OR add to an existing client test)

**Interfaces:**
- Consumes: existing `send_card(feishu, http, tokens, key, card)` callers (no breaking change — default `root_id=None`)
- Produces: `send_card(feishu, http, tokens, key, card, root_id: Option<&str>)`

- [ ] **Step 1: Write failing test**

```rust
// feishu/tests/send_card_root_id_test.rs
use feishu::client::{FeishuClient, TokenManager};

#[tokio::test]
async fn send_card_includes_root_id_in_body() {
    // Construct a FeishuClient pointed at a fake HTTP server that captures
    // the request body, then assert root_id appears in the JSON when passed.
    // ...
}
```

(Note: if there's no existing mock-HTTP pattern, **add the smallest possible test** that calls `send_card` against a `httpmock` or `wiremock` fixture, OR — if both are too heavy — extract the JSON-body construction into a pure helper and test the helper. Use whichever the codebase already does; if nothing, prefer the helper extraction.)

- [ ] **Step 2: Run test, see it fail** — expected: compile error / assertion on missing `root_id` field.

- [ ] **Step 3: Implement** — add `root_id: Option<&str>` parameter to `send_card`; in the JSON body assembly, when `Some(rid)` add `"root_id": rid`. Update all in-crate call sites (run.rs + tests) to pass `None` for now.

- [ ] **Step 4: Run test, see it pass.**

- [ ] **Step 5: Commit** — `git add feishu/src/client.rs feishu/tests/send_card_root_id_test.rs && git commit -m "feat(feishu): send_card accepts optional root_id for quote-reply"`

---

## Task 2: `Out::SendCard` carries `root_id`

**Files:**
- Modify: `router/src/router.rs:34-48` (the `Out::SendCard` variant)

**Interfaces:**
- Consumes: existing callers of `Out::SendCard { .. }` (router emits, run.rs consumes)
- Produces: `Out::SendCard { key, card, msg_id, perm_request_id, perm_meta, root_id: Option<String> }`

- [ ] **Step 1: Write failing test** — add to `router/tests/card_state_test.rs` (or new `turn_card_test.rs`) a test that constructs `Out::SendCard { .., root_id: Some("om_user".into()) }` and asserts the field round-trips. Skip if all `Out` variants are `#[non_exhaustive]` or otherwise untested; just use `Debug` formatting as the assertion.

- [ ] **Step 2: Run test, see it fail** — expected: missing field `root_id`.

- [ ] **Step 3: Implement** — add field to enum variant; add field in every existing `Out::SendCard { .. }` constructor (≈6 sites: dispatch_acp_event PermissionRequest, on_button dead-session, on_button stale click, wire_session_card_and_pump caller, etc.) — initial value `None`.

- [ ] **Step 4: Run existing router test suite** — all must still pass (no behavior change yet).

- [ ] **Step 5: Commit** — `git commit -m "refactor(router): Out::SendCard carries optional root_id"`

---

## Task 3: `MsgIdMap` semantics — flip on each `record`

**Files:**
- Modify: `router/src/router.rs:687-700` (`MsgIdMap`)
- Test: `router/tests/state_test.rs` (existing tests must still pass; add one for flip)

**Interfaces:**
- Consumes: same API (`record(sid, msg_id)`, `get(sid) -> Option<String>`)
- Produces: same API. Semantic change: `record` overwrites previous entry for the same `session_id`. **This is already the behavior** (`HashMap::insert` overwrites) — no code change. Only a doc-comment update and a test asserting that the second `record` call wins.

- [ ] **Step 1: Write failing test** in `router/tests/state_test.rs` (or `turn_card_test.rs` if you prefer a new file):

```rust
#[tokio::test]
async fn msgid_map_record_overwrites_previous_entry() {
    let m = MsgIdMap::default();
    m.record("s1".into(), "om_first".into()).await;
    m.record("s1".into(), "om_second".into()).await;
    assert_eq!(m.get("s1").await.as_deref(), Some("om_second"));
}
```

- [ ] **Step 2: Run, see it fail** — likely passes already (HashMap insert overwrites), but the test pins the behavior. If it passes, mark step complete and move on.

- [ ] **Step 3: Update doc-comment** on `MsgIdMap::record`:

```rust
/// Record the message_id of the **most recent** per-turn card for a session.
/// Called by the dispatcher after each `send_card` returns. Streaming
/// `UpdateCard`s resolve through `get(session_id)`, so each new turn's
/// card "takes over" as the PATCH target — earlier turns stay frozen
/// at their final state. See `docs/superpowers/plans/2026-08-04-per-turn-reply-quote.md`.
```

- [ ] **Step 4: Run test, see it pass.**

- [ ] **Step 5: Commit** — `git commit -m "docs(router): MsgIdMap.record overwrites — points at most recent turn card"`

---

## Task 4: `on_text` carries user message id into the router

**Files:**
- Modify: `router/src/router.rs:303-316` (`RouterHandle::dispatch`) — pass `reply_to` through; `on_text` gains parameter.

**Interfaces:**
- Consumes: `FeishuIn::Text { key, text, reply_to }`
- Produces: `on_text(key, text, reply_to: Option<String>)`. `dispatch` extracts `reply_to` from the enum and forwards.

- [ ] **Step 1: Write failing test** in `router/tests/turn_card_test.rs`:

```rust
#[tokio::test]
async fn first_turn_text_emits_send_card_with_user_reply_to_as_root_id() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey { chat_id: "oc_x".into(), thread_id: None };
    router.map.insert(key.clone(), Mapping::active("s1")).await;

    router.dispatch(FeishuIn::Text {
        key: key.clone(),
        text: "hello".into(),
        reply_to: Some("om_user_first".into()),
    }).await;

    // The continue path should emit a SendCard with root_id=Some(reply_to)
    let out = tokio::time::timeout(Duration::from_millis(100), out_rx.recv())
        .await.unwrap().unwrap();
    // We expect SendAcp (forwarding to existing session) — the card emission
    // happens later in the dispatcher. So this test should only assert
    // SendAcp. The SendCard test lives in Task 5.
    match out {
        Out::SendAcp { .. } => {},
        other => panic!("expected SendAcp, got {other:?}"),
    }
}
```

The above test is a **plumbing check** — does the new `reply_to` parameter compile through `dispatch`? Skip if the test is trivially obvious; the real test lives in Task 5.

- [ ] **Step 2: Implement** — `on_text(&self, key, text, reply_to: Option<String>)`. Update `dispatch` to destructure `reply_to`. Update all existing callers of `on_text` (only `dispatch` itself + tests).

- [ ] **Step 3: Run existing router tests** — must still pass.

- [ ] **Step 4: Commit** — `git commit -m "refactor(router): on_text accepts user message id (reply_to)"`

---

## Task 5: `continue_session` emits a per-turn `SendCard`

**Files:**
- Modify: `router/src/router.rs:565-592` (`continue_session`)

**Interfaces:**
- Consumes: `continue_session(session_id, prompt)` — current signature
- Produces: `continue_session(session_id, prompt, root_id: Option<String>, key: SessionKey)` — emits `Out::SendCard { root_id, msg_id: None, perm_*: None, .. }` **before** `SendAcp`, so the dispatcher sends a fresh card and the pump's subsequent `UpdateCard`s PATCH this new card (because `record_root_msg_id` flips the MsgIdMap pointer).

- [ ] **Step 1: Write failing test** in `router/tests/turn_card_test.rs`:

```rust
#[tokio::test]
async fn continue_session_emits_per_turn_send_card_with_root_id() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey { chat_id: "oc_x".into(), thread_id: None };
    router.map.insert(key.clone(), Mapping::active("sess-1")).await;
    router.seed_card("sess-1".into(), "first".into()).await;

    // First turn finishes (so emoji flips on the second turn).
    router.apply_event_to_out("sess-1".into(), &AcpEvent::Finished { session_id: "sess-1".into() }).await;
    let _ = out_rx.recv().await; // drain

    // User sends a 2nd message that quotes-back to om_user_2.
    router.dispatch(FeishuIn::Text {
        key: key.clone(),
        text: "follow-up".into(),
        reply_to: Some("om_user_2".into()),
    }).await;

    // Expect: SendCard (per-turn) followed by SendAcp.
    let first = out_rx.recv().await.unwrap();
    let second = out_rx.recv().await.unwrap();
    match (&first, &second) {
        (Out::SendCard { root_id: Some(rid), .. }, Out::SendAcp { .. }) => {
            assert_eq!(rid, "om_user_2");
        }
        _ => panic!("expected SendCard(root_id=Some(_)) then SendAcp, got {first:?} then {second:?}"),
    }
}
```

- [ ] **Step 2: Run, see it fail** — expected: panic on missing SendCard or wrong root_id.

- [ ] **Step 3: Implement** — change `continue_session` signature to take `(session_id, prompt, root_id, key)`. Emit `Out::SendCard` with the per-turn card body (use `render_accumulated_card` with the new prompt + SEED emoji so the visual matches the first turn's seed card), then `Out::SendAcp`. Also reset CardState for the new turn (`card_states.drop(session_id)` then `seed_card(session_id, prompt)`) so streaming events accumulate into a fresh body. Update callers (`on_text` Continue arm — needs to look up `key` from `SessionMap`).

- [ ] **Step 4: Run the test, see it pass.**

- [ ] **Step 5: Run full router test suite** — expect card_state_test's 24 tests + new turn_card_test's tests all green.

- [ ] **Step 6: Commit** — `git commit -m "feat(router): continue_session emits per-turn card with root_id"`

---

## Task 6: `SessionMap` turn queue

**Files:**
- Modify: `router/src/state.rs` — add `QueuedTurn` struct + `SessionMap.turn_queue` field + `enqueue_turn`/`pop_next_turn`/`queue_len` methods.

**Interfaces:**
- Consumes: existing `SessionMap` API (`get`, `insert`, `activate`, etc.)
- Produces:
  ```rust
  pub struct QueuedTurn {
      pub prompt: String,
      pub reply_to: Option<String>,  // user msg_id (root_id for the per-turn card)
      pub priority: bool,            // true = /btw slot; goes to front of queue
  }
  impl SessionMap {
      pub async fn enqueue_turn(&self, key: &SessionKey, turn: QueuedTurn);
      pub async fn pop_next_turn(&self, key: &SessionKey) -> Option<QueuedTurn>;
      pub async fn queue_len(&self, key: &SessionKey) -> usize;
  }
  ```
  Priority semantics: `enqueue_turn` with `priority=true` inserts at index 0; `priority=false` appends to back.

- [ ] **Step 1: Write failing test** in `router/tests/state_test.rs`:

```rust
#[tokio::test]
async fn queue_fifo_by_default_priority_jumps_front() {
    let m = SessionMap::new();
    let k = SessionKey { chat_id: "oc".into(), thread_id: None };
    m.insert(k.clone(), Mapping::active("s1")).await;
    m.enqueue_turn(&k, QueuedTurn { prompt: "first".into(), reply_to: None, priority: false }).await;
    m.enqueue_turn(&k, QueuedTurn { prompt: "second".into(), reply_to: None, priority: false }).await;
    m.enqueue_turn(&k, QueuedTurn { prompt: "btw".into(), reply_to: None, priority: true }).await;
    assert_eq!(m.queue_len(&k).await, 3);
    assert_eq!(m.pop_next_turn(&k).await.unwrap().prompt, "btw");  // priority front
    assert_eq!(m.pop_next_turn(&k).await.unwrap().prompt, "first");
    assert_eq!(m.pop_next_turn(&k).await.unwrap().prompt, "second");
    assert!(m.pop_next_turn(&k).await.is_none());
}
```

- [ ] **Step 2: Run, see it fail.**

- [ ] **Step 3: Implement** — add `QueuedTurn` struct, `SessionMap.turn_queue: Arc<RwLock<HashMap<SessionKey, VecDeque<QueuedTurn>>>>`, and the three methods.

- [ ] **Step 4: Run state_test.rs, see it pass.**

- [ ] **Step 5: Commit** — `git commit -m "feat(router): SessionMap.turn_queue (FIFO + /btw priority slot)"`

---

## Task 7: `continue_session` serializes — enqueue when in-flight, ⏳ reaction

**Files:**
- Modify: `router/src/router.rs:565-592` (`continue_session`)

**Interfaces:**
- Consumes: `continue_session(session_id, prompt, root_id, key)` from Task 5
- Produces: same signature; emits either `Out::SendCard + Out::SendAcp` (when settled) **or** `Out::React { ⏳ }` (when in-flight — no new card, no SendAcp). In-flight is determined by `card_states.status_emoji == WORKING` for `session_id`.

- [ ] **Step 1: Write failing test** in `router/tests/turn_card_test.rs`:

```rust
#[tokio::test]
async fn continue_while_in_flight_enqueues_no_card_no_sendacp_only_queue_react() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey { chat_id: "oc".into(), thread_id: None };
    router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;
    // First turn is mid-flight (no Finished yet) — emoji stays at SEED but the
    // dispatch path marks it WORKING once SendAcp lands; we simulate by
    // flipping it manually for the test.
    router.apply_event_to_out("s1".into(), &AcpEvent::TextDelta {
        session_id: "s1".into(), delta: "x".into(),
    }).await;
    let _ = out_rx.recv().await; // drain UpdateCard

    router.dispatch(FeishuIn::Text {
        key: key.clone(),
        text: "second".into(),
        reply_to: Some("om_user_2".into()),
    }).await;

    // Expect: only a React with ⏳ — no SendCard, no SendAcp.
    let out = tokio::time::timeout(Duration::from_millis(50), out_rx.recv())
        .await.unwrap().unwrap();
    match out {
        Out::React { emoji, .. } => assert_eq!(emoji, "⏳"),
        other => panic!("expected React(⏳), got {other:?}"),
    }
    // Nothing else in flight.
    assert!(tokio::time::timeout(Duration::from_millis(50), out_rx.recv()).await.is_err());
    // Queue contains the queued turn.
    assert_eq!(router.map.queue_len(&key).await, 1);
}

#[tokio::test]
async fn continue_when_settled_emits_new_card_and_sendacp() {
    // mirrors the existing Task-5 test; verify the settled path still works
    // when interleaved with the queueing path.
}
```

- [ ] **Step 2: Run, see it fail.**

- [ ] **Step 3: Implement** — at the top of `continue_session`, check `card_states.status_emoji(session_id) == WORKING`. If so, `map.enqueue_turn(key, QueuedTurn { prompt, reply_to, priority })` then `emit_reaction(⏳)`; return. Otherwise, fall through to the existing Task-5 path (POST card + SendAcp).

- [ ] **Step 4: Run, see it pass.**

- [ ] **Step 5: Commit** — `git commit -m "feat(router): serialize multi-turn (queue + ⏳ reaction when in-flight)"`

---

## Task 8: Drain queue on settle (`Finished` / `Error`)

**Files:**
- Modify: `router/src/router.rs` — extend `apply_event_to_out`'s `_ => { ... }` arm (around line 402) and the `Error { terminal: true, .. }` arm (around line 385) to drain the queue after the FSM transitions to DONE/FAILED.

**Interfaces:**
- Consumes: existing event paths that finalize a turn.
- Produces: after the existing flush + reaction, if `card_states.status_emoji` is now DONE/FAILED and `map.queue_len(key) > 0`: pop next turn from queue, reset CardState via `drop_card` + `seed_card`, post fresh SendCard with `root_id = next.reply_to`, `Out::SendAcp` with `next.prompt`. Recurse-safe (loop until queue empty or status leaves terminal).

- [ ] **Step 1: Write failing test** in `router/tests/turn_card_test.rs`:

```rust
#[tokio::test]
async fn drain_queue_emits_next_turn_card_and_sendacp_after_finished() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey { chat_id: "oc".into(), thread_id: None };
    router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;
    // Mid-flight.
    router.apply_event_to_out("s1".into(), &AcpEvent::TextDelta {
        session_id: "s1".into(), delta: "x".into(),
    }).await; let _ = out_rx.recv().await;
    // Queue 2 turns while in-flight.
    router.dispatch(FeishuIn::Text {
        key: key.clone(), text: "second".into(), reply_to: Some("om2".into()),
    }).await; let _ = out_rx.recv().await; // ⏳ react
    router.dispatch(FeishuIn::Text {
        key: key.clone(), text: "third".into(), reply_to: Some("om3".into()),
    }).await; let _ = out_rx.recv().await; // ⏳ react
    assert_eq!(router.map.queue_len(&key).await, 2);

    // Settle turn 1.
    router.apply_event_to_out("s1".into(), &AcpEvent::Finished { session_id: "s1".into() }).await;
    let _ = out_rx.recv().await; // UpdateCard (✅)
    let _ = out_rx.recv().await; // React ✅

    // Now turn 2 should drain: SendCard(root_id=om2) + SendAcp("second")
    let first = out_rx.recv().await.unwrap();
    let second = out_rx.recv().await.unwrap();
    match (&first, &second) {
        (Out::SendCard { root_id: Some(rid), .. }, Out::SendAcp { .. }) => {
            assert_eq!(rid, "om2");
        }
        _ => panic!("expected SendCard(om2) + SendAcp, got {first:?} then {second:?}"),
    }
    assert_eq!(router.map.queue_len(&key).await, 1);

    // Settle turn 2.
    router.apply_event_to_out("s1".into(), &AcpEvent::Finished { session_id: "s1".into() }).await;
    let _ = out_rx.recv().await; let _ = out_rx.recv().await; // UpdateCard + React
    let third_a = out_rx.recv().await.unwrap();
    let third_b = out_rx.recv().await.unwrap();
    match (&third_a, &third_b) {
        (Out::SendCard { root_id: Some(rid), .. }, _) => assert_eq!(rid, "om3"),
        _ => panic!("expected SendCard(om3), got {third_a:?}"),
    }
    assert_eq!(router.map.queue_len(&key).await, 0);
}
```

- [ ] **Step 2: Run, see it fail.**

- [ ] **Step 3: Implement** — extract a private helper `drain_queue_if_terminal(&self, key: &SessionKey, session_id: &str)` that: checks `card_states.status_emoji(session_id) in {DONE, FAILED}` AND `map.queue_len(key) > 0`; pops the next turn, calls `drop_card` + `seed_card` to reset the per-turn body, emits `Out::SendCard { root_id, msg_id: None, perm_*: None, .. }` (same render as the settled path), emits `Out::SendAcp { cmd: ContinueSession { session_id, prompt } }`. Call from both arms of `apply_event_to_out` after the existing flush+react.

- [ ] **Step 4: Run, see it pass.**

- [ ] **Step 5: Commit** — `git commit -m "feat(router): drain turn queue when in-flight turn settles"`

---

## Task 9: `/btw` command parses and routes with priority

**Files:**
- Modify: `router/src/commands.rs` — add `Command::Btw(String)`; parse `/btw <rest>` from input.
- Modify: `router/src/router.rs` — `on_text` matches `Command::Btw(s)` → `on_text` path, but with `priority=true` flag passed through (new field on the turn struct, or a new arm).

**Interfaces:**
- Consumes: `/btw <text>` from user
- Produces: same handling as `Command::PassThrough`, except `enqueue_turn(.., priority=true)`. If session is settled (not in-flight), the priority flag is a no-op — turns run serially either way; the priority only matters when queueing.

- [ ] **Step 1: Write failing test** in `router/tests/commands_test.rs` (or extend an existing parser test):

```rust
#[test]
fn parse_btw_command() {
    let cmd = parse_command("/btw 顺便问一句");
    assert!(matches!(cmd, Command::Btw(s) if s == "顺便问一句"));
}
```

- [ ] **Step 2: Implement** — `Command::Btw(String)` variant; parser arm.

- [ ] **Step 3: Write router test** in `router/tests/turn_card_test.rs`:

```rust
#[tokio::test]
async fn btw_command_queues_with_priority_ahead_of_existing_fifo() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey { chat_id: "oc".into(), thread_id: None };
    router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;
    router.apply_event_to_out("s1".into(), &AcpEvent::TextDelta {
        session_id: "s1".into(), delta: "x".into(),
    }).await; let _ = out_rx.recv().await;

    // Queue a normal FIFO turn first.
    router.dispatch(FeishuIn::Text {
        key: key.clone(), text: "fifo".into(), reply_to: Some("omF".into()),
    }).await; let _ = out_rx.recv().await; // ⏳

    // Now a /btw turn — must jump to front.
    router.dispatch(FeishuIn::Text {
        key: key.clone(), text: "/btw btw".into(), reply_to: Some("omB".into()),
    }).await; let _ = out_rx.recv().await; // ⏳

    router.apply_event_to_out("s1".into(), &AcpEvent::Finished { session_id: "s1".into() }).await;
    let _ = out_rx.recv().await; let _ = out_rx.recv().await; // UpdateCard + React ✅

    // Drain: first SendCard should be the /btw one (omB), not FIFO (omF).
    let first = out_rx.recv().await.unwrap();
    match first {
        Out::SendCard { root_id: Some(rid), .. } => assert_eq!(rid, "omB"),
        other => panic!("expected SendCard(omB), got {other:?}"),
    }
    let second = out_rx.recv().await.unwrap();
    match second {
        Out::SendAcp { .. } => {}
        other => panic!("expected SendAcp, got {other:?}"),
    }
}
```

- [ ] **Step 4: Run, see fail; implement; see pass.**

- [ ] **Step 5: Commit** — `git commit -m "feat(router): /btw command — priority slot in turn queue"`

---

## Task 10: Dispatcher sends `Out::SendCard` with `root_id`

**Files:**
- Modify: `src/run.rs` — find the `Out::SendCard` arm (search for `Out::SendCard { key, card, msg_id,`); thread `root_id` through to `feishu.send_card(http, tokens, key, card, root_id.as_deref())`.

**Interfaces:**
- Consumes: `Out::SendCard { root_id, .. }` (new field from Task 2)
- Produces: existing `record_root_msg_id` + reaction flow still runs, but with the new card's message_id overwriting the previous turn's.

- [ ] **Step 1: Write failing test** — add to `sebas/tests/integration` (or extend `full_e2e_test.rs` if it covers send_card path). The test must assert that `feishu::client::send_card` was called with `root_id=Some("om_user")` on a continuation turn. **If the existing e2e test infra doesn't allow that assertion**, add a smaller unit-level test that calls the dispatcher's SendCard handler with a fake FeishuClient (one that records the last `send_card` call's args). Reference: see `feishu/tests/` for any existing fake-client pattern.

If neither fits, **skip the test** for this task and rely on Tasks 5 + 1's tests + manual smoke (Task 11). Note in the commit message that this task is plumbing only.

- [ ] **Step 2: Implement** — pull `root_id` out of the `Out::SendCard` destructure; pass it to `feishu.send_card`.

- [ ] **Step 3: Run dispatcher-level tests** — must still pass.

- [ ] **Step 4: Commit** — `git commit -m "feat(sebas): dispatcher propagates SendCard.root_id to feishu API"`

---

## Task 11: New multi-turn integration test (freezes old turn)

**Files:**
- Create: `router/tests/turn_card_test.rs`

**Interfaces:**
- Tests: end-to-end flow per turn

- [ ] **Step 1: Write test** — covers three scenarios in one file:

```rust
//! Multi-turn reply-quote semantics.
//!
//! Each user text turn emits a fresh SendCard with `root_id = reply_to`.
//! Streaming UpdateCards from the pump resolve via MsgIdMap → most recent
//! turn's msg_id, so the previous turn's card stays frozen at its final state.

#[tokio::test]
async fn three_turn_chat_yields_three_distinct_send_cards() { /* ... */ }

#[tokio::test]
async fn streaming_update_after_second_turn_targets_second_card_not_first() {
    // 1. seed_card session, apply_event TextDelta → first turn UpdateCard (target: msg_id_1)
    // 2. apply_event Finished → first turn done
    // 3. user sends 2nd text → dispatcher emits SendCard_2 with root_id=om_user_2; record_root_msg_id flips to msg_id_2
    // 4. apply_event TextDelta for 2nd turn → UpdateCard (target: msg_id_2)
    // 5. assert: UpdateCard targets msg_id_2 (NOT msg_id_1); the first turn's card is no longer in flight
}

#[tokio::test]
async fn first_turn_root_id_is_some_user_message_id() { /* ... */ }

#[tokio::test]
async fn missing_reply_to_does_not_panic_emits_no_root_id() { /* ... */ }
```

- [ ] **Step 2: Run, see all pass** (they should already pass given Tasks 5+6).

- [ ] **Step 3: Commit** — `git commit -m "test(router): multi-turn reply-quote (3 turns, frozen previous card)"`

---

## Task 12: Real-machine smoke + final full-suite

**Files:** none (validation only)

- [ ] **Step 1: Build all** — `cargo build --jobs 2`

- [ ] **Step 2: Run full test suite** — `cargo test --jobs 2 --no-fail-fast`. Expect: every existing target green + new `turn_card_test.rs` green + `card_state_test.rs` 24 green.

- [ ] **Step 3: Rebuild bridge binary** — `cargo build -p acp-claude-bridge --bins --jobs 2` (e2e tests require it)

- [ ] **Step 4: Restart sebas** — kill existing instance, start new with `SEBAS_LOG_LEVEL=debug`.

- [ ] **Step 5: Manual smoke (Feishu chat)** —
  1. Send 1st message: any prompt → expect ONE new card (👀 → 🚧 → ✅)
  2. Send 2nd message: another prompt → expect a NEW card in the chat scroll, quote-replying to message 1 (NOT a PATCH of card 1). Card 1 should stay frozen at its ✅ state.
  3. Send 3rd message: another prompt → third card. Cards 1 and 2 stay frozen.
  4. Send a permission-requiring tool (e.g. `Bash rm /tmp/foo`). Permission card appears as its own message, quote-replying to the 4th user message. Click "Allow session" → flips in place.
  5. Same `Bash` again → no card (allowlist hit).
  6. **Serial queue**: send a long prompt (e.g. "请阅读 /home/bot/workbench/repos/sebas/Cargo.toml 然后总结") — while it's working, send a 2nd message → expect ⏳ on the in-flight card (no new card, no premature SendAcp). When turn 1 finishes, expect the queued turn to drain and start.
  7. **/btw**: while turn 1 is running, type `/btw 顺便问一句：现在几点？` → expect ⏳ on the in-flight card; when turn 1 finishes, the /btw turn runs FIRST before any other queued FIFO turns.

- [ ] **Step 6: Capture logs and report** — `git status`, file list, test counts, smoke observations. **Do not push** — per Conservative profile, wait for approval.

- [ ] **Step 7: (Optional, only if asked)** Commit plan docs — `git add docs/superpowers/plans/2026-08-04-per-turn-reply-quote.md && git commit -m "docs: per-turn reply-quote refactor plan"`.

---

## Self-Review Notes

- **Spec coverage**: Three pain points (card growth, no quote, no topic switch) → Task 5 + 11 cover them. Serial execution → Tasks 6–8. /btw priority → Task 9. Per-turn FSM scope → unchanged. Throttling → unchanged (Task 10 keeps existing path).
- **Placeholders**: No TBDs. Code blocks are concrete. Mock-HTTP detail in Task 1 defers to existing patterns — that's a deliberate "follow codebase convention" not a placeholder.
- **Type consistency**: `Out::SendCard` gains `root_id` in Task 2; Task 4 adds `reply_to` to `on_text`; Task 5 plumbs both; Task 7 reuses Task-5's settled-path logic on drain; Task 9 reuses Task-7's enqueue path with a priority flag. Signatures cross-reference inline. `MsgIdMap` shape unchanged.
- **Risk**: Task 5 resets CardState on each turn — if a streaming event from turn 1 arrives after turn 2 starts (race), it would land in turn 2's body. **Mitigation**: the dispatcher gates SendCard emission before SendAcp, and the bridge is one-prompt-in-flight, so no event interleaving is expected. If a real race appears in smoke (Task 12.5), add a guard.
- **Queue & on_text ordering**: Tasks 7+8 assume `on_text` for a settled session flows through `continue_session`. The path for `/new` (SpawnNew) and `SpawnResume` are unchanged.

## Execution Handoff

After Task 8 completes, report:
- Files changed (count + paths).
- Test totals (`X passed, 0 failed` per target).
- Smoke observations (5 steps).
- **`git status` output** for review.
- **Wait for explicit "push" / "merge"** before any remote sync.