## ADDED Requirements

### Requirement: Runtime state boundaries for persisted session state

The state store SHALL NOT persist the permission allowlist, outstanding permission cards, card states, or in-flight spawn placeholders — these SHALL be reconstructed or re-prompted after a restart. The agent session map SHALL be persisted in the state store and written per mutation, rather than only at daemon shutdown, so an unclean core termination loses at most the mutations committed after the last successful response.

#### Scenario: Allowlist survives no restart

- **WHEN** the daemon restarts
- **THEN** previously granted session-scoped permissions are gone and the user is prompted again

#### Scenario: Spawn placeholders are never written

- **WHEN** the daemon shuts down while a spawn is in flight
- **THEN** the persisted state contains no trace of the in-flight spawn

#### Scenario: Session map survives unclean exit

- **WHEN** core is killed while sessions are active
- **THEN** the session map at next start reflects the last committed state, not the last graceful shutdown
