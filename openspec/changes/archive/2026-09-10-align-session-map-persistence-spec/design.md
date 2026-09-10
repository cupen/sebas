## Context

`state-store`「Runtime state boundaries」over-promised session-map durability (state store, per-mutation, survives unclean exit) that the `add-state-store` design explicitly deferred. Code persists the session map to `[dispatch] state_file` (JSON) at graceful shutdown only; the DB `session_map` table is a reserved placeholder whose field shape (`chat_id/thread_id`) predates the neutral `ChannelKey`→DTO mapping and cannot store `acp_session_id`/`current_model`/`pending_kind`. The `session-lifecycle`「Restart recovery」requirement already describes the JSON-file mechanism honestly, so the two specs contradicted each other.

## Goals / Non-Goals

**Goals:**
- Make `state-store` honestly describe current session-map persistence (shutdown-only JSON) and mark the DB migration as deferred.

**Non-Goals:**
- No code change.
- Do not delete the reserved `session_map` table.
- Do not implement per-mutation DB persistence here.

## Decisions

- Point `state-store` at `session-lifecycle`「Restart recovery」as the authoritative source for session-map persistence, and record the deferred-migration pointer (mapping-change-owned table shape). This resolves the internal spec contradiction without touching behavior.

## Risks / Trade-offs

- Acknowledging the deferred migration removes a durability guarantee some readers may have assumed; the honest scenario (survives only graceful-shutdown snapshot) is the truthful contract. The future mapping change restores the stronger guarantee.
