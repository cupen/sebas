# Permission flow — sequence diagrams

The full permission round-trip from claude's tool_use to Feishu's card flip.

> Post-ACP architecture (sebas-dk8): sebas drives `claude` directly via
> `cc-agent-sdk` (stream-json + control protocol over stdio). The bridge
> process, the PreToolUse hook script and the unix-socket broker are gone —
> the permission gate is a process-internal hook callback correlated by
> control `request_id`. Design: `docs/superpowers/specs/2026-08-06-claude-direct-sdk-refactor-design.md` §4.2.

**Participants**
- **Claude** — `claude` CLI child (stream-json + control protocol on stdio)
- **SDK** — `cc-agent-sdk` `ClaudeClient` inside sebas (owns the child)
- **Sebas** — daemon: acp-claude driver/manager + router + Feishu client
- **Feishu** — chat platform
- **User** — human at Feishu

---

## 1. First-time prompt (allowlist miss → card → click)

The common case: the (tool, args) signature is NOT on the per-chat
allowlist, so the user must click.

```mermaid
sequenceDiagram
    autonumber
    participant Claude
    participant SDK as cc-agent-sdk (in sebas)
    participant Sebas as sebas (driver+router)
    participant Feishu
    participant User

    Claude->>SDK: control_request hook_callback(request_id=req-1, PreToolUse)
    Note over SDK: dispatches to the driver's PreToolUse hook callback

    SDK->>Sebas: permission_hook(input, tool_use_id=req-1)
    Note over Sebas: driver.rs — parks oneshot in pending_perms[req-1]
    Sebas->>Sebas: emit AcpEvent::PermissionRequest{session, req-1, tool, args}

    Note over Sebas: router.rs — apply_event_to_out
    Sebas->>Sebas: allowlist.is_allowed(key, tool, args)?
    Note right of Sebas: MISS — first time this chat sees this call

    Sebas->>Feishu: SendCard(permission card, perm_request_id=req-1, perm_meta=(tool,args))
    Feishu-->>User: card with [Allow once] [Allow session] [Deny]

    Note over Sebas: run.rs — dispatch_out
    Sebas->>Feishu: send_card → returns msg_id
    Sebas->>Sebas: perm_cards[req-1] = (key, msg_id, tool, args)

    User->>Feishu: click "Allow session"
    Feishu->>Sebas: card.action.trigger{decision=allow_session, request_id=req-1}

    Note over Sebas: router.rs — on_button
    Sebas->>Sebas: perm_cards.take(req-1) → entry
    alt decision = AllowSession
        Sebas->>Sebas: allowlist.grant(key, tool, args)
    end
    Sebas->>Feishu: UpdateCardByMsgId(msg_id, "已允许（本会话）")
    Note right of Feishu: card flips in place
    Sebas->>Sebas: SendAcp { PermissionReply(AllowSession) }
    Note over Sebas: manager.rs — send() resolves oneshot pending_perms[req-1]

    Sebas-->>SDK: hook callback returns HookJsonOutput{permissionDecision: allow}
    SDK-->>Claude: control_response(request_id=req-1)
    Note over Claude: tool runs
```

The callback blocks on the oneshot the whole time — correlation is by
`request_id` (the claude `tool_use_id`), never by position. Parallel tool
calls each get their own request id and their own parked oneshot.

---

## 2. Auto-approve (allowlist hit)

The (tool, args) was previously granted with "Allow session" in this
chat. The hook callback still runs, but the user sees nothing — no card,
no click needed.

```mermaid
sequenceDiagram
    autonumber
    participant Claude
    participant SDK as cc-agent-sdk (in sebas)
    participant Sebas
    participant User

    Claude->>SDK: control_request hook_callback(request_id=req-N)
    SDK->>Sebas: permission_hook(input, tool_use_id=req-N)
    Note over Sebas: parks oneshot pending_perms[req-N]
    Sebas->>Sebas: emit AcpEvent::PermissionRequest

    Note over Sebas: apply_event_to_out
    Sebas->>Sebas: allowlist.is_allowed(key, tool, args)?
    Note right of Sebas: HIT — exact signature match
    Sebas->>Sebas: skip SendCard
    Sebas->>Sebas: SendAcp { PermissionReply(AllowSession) }
    Note over Sebas: manager.send resolves oneshot[req-N]

    Sebas-->>SDK: HookJsonOutput{permissionDecision: allow}
    SDK-->>Claude: control_response(request_id=req-N)
    Note over User: nothing shown — tool just runs
```

---

## 3. Stale click (user clicks an already-resolved card)

The card stayed in Feishu after the first click. A follow-up click
hits the "no live perm card" path and shows a "已过期" card so the
user knows the click had no effect.

```mermaid
sequenceDiagram
    autonumber
    participant Feishu
    participant Sebas
    participant User

    Note over Feishu,Sebas: state from PATH 1<br/>perm_cards[req-1] was taken (None now)

    User->>Feishu: click "Allow once" on the resolved card
    Feishu->>Sebas: card.action.trigger{decision=allow_once, request_id=req-1}

    Note over Sebas: router.rs — on_button
    Sebas->>Sebas: perm_cards.take(req-1) → None
    Note right of Sebas: stale — entry already consumed
    Sebas->>Feishu: SendCard("⚠ 请求已过期")
    Feishu-->>User: new "已过期" card
    Note over Sebas: NO PermissionReply — no oneshot to resolve
```

If a stale/unknown `request_id` ever reaches `manager.send` directly, the
send is logged and dropped (`no pending responder`) — fail closed: the
hook callback's dropped-oneshot path answers `deny`.

---

## 4. Session end (allowlist cleared)

When a claude session terminates (terminal error, `/new`, daemon
restart) the per-chat allowlist is wiped so a new session starts
fresh.

```mermaid
sequenceDiagram
    autonumber
    participant Claude
    participant Sebas

    Claude-->>Sebas: process exit → driver watchdog/EOF → AcpEvent::Error{terminal: true}

    Note over Sebas: router.rs — terminal branch
    Sebas->>Sebas: apply_event (set FAILED + append body)
    Sebas->>Sebas: flush_card
    Sebas->>Sebas: emit_reaction(FAILED)
    Sebas->>Sebas: lookup_key_by_session(session_id)
    Sebas->>Sebas: allowlist.clear(key) ← wipes per-chat memory
    Sebas->>Sebas: map.remove_by_session(session_id)
    Sebas->>Sebas: card_states.drop(session_id)

    Note over Sebas: next /new → fresh mapping, fresh allowlist
```

---

## Key invariants

- **One oneshot per request_id.** The driver's hook callback parks the
  decision oneshot in `pending_perms`; the router only sends a
  `PermissionReply` when the user actually clicks (or auto-approves).
  `oneshot::Sender` gives exact FnOnce semantics — a request can be
  answered at most once.
- **Correlation is explicit, never positional.** The hook callback's
  control `request_id` (= claude's `tool_use_id`) keys the oneshot map;
  parallel tool calls cannot mispair (the ACP-era broker FIFO hazard is
  gone with the broker).
- **Fail closed.** If the router is gone or nobody answers, the oneshot
  resolves to `deny`; an unanswered callback never auto-allows.
- **`take_perm_card` removes the entry on click.** A duplicate click
  sees `None` → "已过期" card, no double reply.
- **Allowlist scope = per `SessionKey`** (chat_id, thread_id). Cleared
  on session end. New `/new` → empty allowlist.
- **Signature is exact match** (`{tool}|{args_json}`). Slightly
  different args → fresh prompt, not auto-approved.
