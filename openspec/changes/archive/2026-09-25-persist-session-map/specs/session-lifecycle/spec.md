## MODIFIED Requirements

### Requirement: Restart recovery with corruption tolerance

On daemon start, the system SHALL restore the persisted session map from the state store: an empty store yields an empty table; restored entries become Dormant. Session-map entries SHALL be written per mutation, so that an unclean exit preserves every committed mapping and shutdown ordering no longer decides what survives. The daemon SHALL never refuse to start because of session-map state itself: a session map whose entries cannot be read is reported honestly and the daemon starts with an empty table. A state store that cannot be opened at all is governed by the state store's corruption rule and SHALL NOT be reset or recreated by session-map recovery.

#### Scenario: Corrupt session map is quarantined

- **WHEN** the persisted session-map entries cannot be read at startup
- **THEN** the unreadable data is set aside rather than presented as valid, with the reason logged
- **AND** the daemon starts with an empty session table
- **AND** a store that cannot be opened at all follows the state store's corruption rule instead (refuse to start with a diagnostic, never reset)

#### Scenario: Missing file starts empty

- **WHEN** no persisted session map exists at startup
- **THEN** the daemon starts with an empty table and no error

#### Scenario: Snapshot precedes shutdown kill

- **WHEN** the daemon shuts down while sessions are active
- **THEN** every mapping committed before shutdown is already durable in the state store
- **AND** shutdown ordering cannot lose a mapping, because no shutdown-time snapshot is required to persist it

#### Scenario: Unclean exit keeps the mapping

- **WHEN** the daemon is killed without a graceful shutdown while sessions are active
- **THEN** after restart the session map contains every mapping committed before the kill
- **AND** it does not depend on a snapshot having been written at the last graceful shutdown

## ADDED Requirements

### Requirement: Session map is persisted per mutation

Each committed change to a session's mapping SHALL be durable in the state store before it is observable to clients, so that an abrupt process exit cannot roll a mapping back to an earlier state. The session map SHALL NOT be persisted only at shutdown.

#### Scenario: A committed mapping survives a kill

- **WHEN** a session is created and its mapping is observable to a client, and the daemon is then killed immediately
- **THEN** after restart the mapping is present with the same session identity and desired mode

#### Scenario: Rolling back a mapping is not observable

- **WHEN** a mapping change is committed and a client then reads the session map
- **THEN** the read reflects the committed change
- **AND** a subsequent abrupt exit does not revert it
