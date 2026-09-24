## MODIFIED Requirements

### Requirement: Runtime state boundaries for persisted session state

The state store SHALL NOT persist the permission allowlist, outstanding permission cards, card states, or in-flight spawn placeholders — these SHALL be reconstructed or re-prompted after a restart. The agent session map SHALL be persisted in the state store and written per mutation, so that an unclean exit preserves every committed mapping; `session-lifecycle`「Restart recovery with corruption tolerance」governs how it is restored, and `session-persistence`「Runtime state is not persisted by this store」governs what is deliberately excluded. The state store SHALL NOT keep a second, shutdown-time snapshot of the session map, and no separate session-map file SHALL be written or read.

#### Scenario: Allowlist survives no restart

- **WHEN** the daemon restarts
- **THEN** previously granted session-scoped permissions are gone and the user is prompted again

#### Scenario: Spawn placeholders are never written

- **WHEN** the daemon shuts down while a spawn is in flight
- **THEN** the persisted state contains no trace of the in-flight spawn

#### Scenario: Session map survives unclean exit

- **WHEN** core is terminated without a graceful shutdown (e.g. SIGKILL) while sessions are active
- **THEN** the session map at next start reflects every mapping committed before the kill, not the state at the last graceful shutdown
- **AND** no shutdown-time snapshot of the session map is required to achieve this

#### Scenario: No separate session-map file exists

- **WHEN** sessions are created, mutated, and closed
- **THEN** no session-map file is created or modified on disk
- **AND** the state store is the only place the session map is persisted
