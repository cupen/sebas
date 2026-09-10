## MODIFIED Requirements

### Requirement: Runtime state boundaries for persisted session state

The state store SHALL NOT persist the permission allowlist, outstanding permission cards, card states, or in-flight spawn placeholders — these SHALL be reconstructed or re-prompted after a restart. The agent session map is currently persisted by the core as a shutdown-only JSON snapshot to `[dispatch] state_file` (see `session-lifecycle`「Restart recovery with corruption tolerance」, the authoritative behavior source); the state store's `session_map` table is a reserved placeholder and does not yet carry sessions. Migrating the session map into the state store is a deferred design step (see the `add-state-store` change runbook): it SHALL be implemented when the session-mapping change lands, using a table shaped to the mapping structure (`ChannelKey` → DTO with `acp_session_id` / `current_model` / `pending_kind`).

#### Scenario: Allowlist survives no restart

- **WHEN** the daemon restarts
- **THEN** previously granted session-scoped permissions are gone and the user is prompted again

#### Scenario: Spawn placeholders are never written

- **WHEN** the daemon shuts down while a spawn is in flight
- **THEN** the persisted state contains no trace of the in-flight spawn

#### Scenario: Session map survives unclean exit

- **WHEN** core is terminated without a graceful shutdown (e.g. SIGKILL)
- **THEN** the session map at next start reflects the snapshot written at the last graceful shutdown, not any in-flight mutations since then (per-mutation durability is a deferred migration, not yet implemented)
