# Permission flow — sequence diagrams

The full permission round-trip from Claude's tool_use to Feishu's card flip.

**Participants**
- **Claude** — `claude --print --verbose` (spawned by bridge)
- **Bridge** — `acp-claude-bridge` (spawned by sebas, speaks ACP over stdio)
- **Sebas** — daemon: router + acp-claude manager + Feishu client
- **Feishu** — chat platform
- **Hook** — Claude's `PreToolUse` hook (sockets to bridge)
- **User** — human at Feishu

---

## 1. First-time prompt (allowlist miss → card → click)

The common case: the (tool, args) signature is NOT on the per-chat
allowlist, so the user must click.

```mermaid
sequenceDiagram
    autonumber
    participant Claude
    participant Bridge
    participant Sebas as sebas (router+manager)
    participant Feishu
    participant User

    Claude->>Bridge: tool_use block (stream-json)
    Note over Bridge: server.rs:196 — ToolUse match arm

    Bridge->>Sebas: session/request_permission(tool_call_id=req-1)
    Note over Sebas: manager.rs:347 — on_receive_request callback
    Sebas->>Sebas: pending_responders[req-1] = responder
    Sebas->>Sebas: emit AcpEvent::PermissionRequest{session, req-1, tool, args}

    Note over Sebas: router.rs:296 — apply_event_to_out
    Sebas->>Sebas: allowlist.is_allowed(key, tool, args)?
    Note right of Sebas: MISS — first time this chat sees this call

    Sebas->>Feishu: SendCard(permission card, perm_request_id=req-1, perm_meta=(tool,args))
    Feishu-->>User: card with [Allow once] [Allow session] [Deny]

    Note over Sebas: run.rs:254 — dispatch_out
    Sebas->>Feishu: send_card → returns msg_id
    Sebas->>Sebas: perm_cards[req-1] = (key, msg_id, tool, args)

    User->>Feishu: click "Allow session"
    Feishu->>Sebas: card.action.trigger{decision=allow_session, request_id=req-1}

    Note over Sebas: router.rs:on_button
    Sebas->>Sebas: perm_cards.take(req-1) → entry
    alt decision = AllowSession
        Sebas->>Sebas: allowlist.grant(key, tool, args)
    end
    Sebas->>Feishu: UpdateCardByMsgId(msg_id, "已允许（本会话）")
    Note right of Feishu: card flips in place
    Sebas->>Bridge: SendAcp { PermissionReply(AllowSession) }
    Sebas->>Sebas: mgr.send → responder[req-1](AllowSession)

    Bridge-->>Claude: decision = approve
    Note over Claude: tool runs
```

---

## 2. Auto-approve (allowlist hit)

The (tool, args) was previously granted with "Allow session" in this
chat. The bridge sees the same approve decision, but the user sees
nothing — no card, no click needed.

```mermaid
sequenceDiagram
    autonumber
    participant Claude
    participant Bridge
    participant Sebas
    participant User

    Claude->>Bridge: tool_use block (same tool, same args)
    Bridge->>Sebas: session/request_permission(req-N)
    Sebas->>Sebas: pending_responders[req-N] = responder
    Sebas->>Sebas: emit AcpEvent::PermissionRequest

    Note over Sebas: apply_event_to_out
    Sebas->>Sebas: allowlist.is_allowed(key, tool, args)?
    Note right of Sebas: HIT — exact signature match
    Sebas->>Sebas: skip SendCard
    Sebas->>Bridge: SendAcp { PermissionReply(AllowSession) }
    Sebas->>Sebas: mgr.send → responder[req-N](AllowSession)

    Bridge-->>Claude: approve
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

    Note over Feishu,Sebas: state from PATH 1 step 18<br/>perm_cards[req-1] was taken (None now)

    User->>Feishu: click "Allow once" on the resolved card
    Feishu->>Sebas: card.action.trigger{decision=allow_once, request_id=req-1}

    Note over Sebas: router.rs:on_button
    Sebas->>Sebas: perm_cards.take(req-1) → None
    Note right of Sebas: stale — entry already consumed
    Sebas->>Feishu: SendCard("⚠ 请求已过期")
    Feishu-->>User: new "已过期" card
    Note over Sebas: NO SendAcp — no responder to call
```

---

## 4. Session end (allowlist cleared)

When a Claude session terminates (terminal error, `/new`, daemon
restart) the per-chat allowlist is wiped so a new session starts
fresh.

```mermaid
sequenceDiagram
    autonumber
    participant Claude
    participant Bridge
    participant Sebas

    Claude-->>Bridge: stream ends with terminal Error
    Bridge->>Sebas: AcpEvent::Error{terminal: true, ...}

    Note over Sebas: router.rs:318 — terminal branch
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

- **One responder per request_id.** The bridge holds the lock; the
  router only sends a `PermissionReply` when the user actually clicks.
  Auto-approve sends the same kind of `PermissionReply` from the router
  directly — the bridge can't tell them apart.
- **`take_perm_card` removes the entry on click.** A duplicate click
  sees `None` → "已过期" card, no double reply.
- **Allowlist scope = per `SessionKey`** (chat_id, thread_id). Cleared
  on session end. New `/new` → empty allowlist.
- **Signature is exact match** (`{tool}|{args_json}`). Slightly
  different args → fresh prompt, not auto-approved.
