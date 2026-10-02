## MODIFIED Requirements

### Requirement: Runtime state is not persisted by this store

The store SHALL NOT persist the permission allowlist, outstanding permission cards, card states, or in-flight spawn placeholders; interactive-session plane state (turn transcripts, parked approvals, per-session usage) is owned by the `session-transcript-durability` capability's checkpoints in the core state store, not by this provider store. The agent session map SHALL be persisted per mutation in the core state store's `session_map` table (the former graceful-shutdown dump-to-file path is retired — not written, not read); an unclean exit therefore loses only in-flight mutations, and consumers SHALL tolerate the loaded map as possibly stale.

#### Scenario: Allowlist survives no restart

- **WHEN** the daemon restarts
- **THEN** previously granted session-scoped permissions are gone and the user is prompted again

#### Scenario: Spawn placeholders are never written

- **WHEN** the daemon shuts down while a spawn is in flight
- **THEN** the persisted state contains no trace of the in-flight spawn

#### Scenario: Session map survives graceful shutdown only

- **WHEN** core is stopped (gracefully or by a hard kill) while sessions exist and then restarts
- **THEN** the session registry reflects the last per-mutation writes in the state store — no state file is dumped or read, and no transcript content is implied by the registry alone
