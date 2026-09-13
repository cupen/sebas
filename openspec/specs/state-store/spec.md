# state-store Specification

## Purpose

Owns the SQLite-backed domain state of sebas: where the database lives, how schema versioning and automatic migration behave, who may write, which state methods the core channel exposes, and the durability and unavailability contracts that replace the former per-file JSON semantics.

## Requirements

### Requirement: Database location and single-writer ownership

The domain state SHALL live in a single SQLite database at `~/.sebas/sebas.db` (overridable via environment variable), opened in WAL mode. Paths SHALL expand a leading `~/`. Only the core process SHALL open the database; all other processes access state exclusively through the core channel state methods. All mutations SHALL be applied by the core state store serialized one at a time.

#### Scenario: Environment override relocates the database

- **WHEN** the database-path environment variable points to a custom path
- **THEN** core opens the database at that path

#### Scenario: Tilde paths expand to home

- **WHEN** a configured database path begins with `~/`
- **THEN** the path is expanded to the user's home directory before use

#### Scenario: Concurrent mutations serialize

- **WHEN** two clients issue state mutations concurrently
- **THEN** both apply in serialization order and a later snapshot reflects the combined result — never a torn or lost update without an explicit error

### Requirement: State methods on the core channel

The core channel SHALL expose state methods for snapshot queries (providers with model aliases, settings, projects, session map) and mutations (provider/alias/settings/projects CRUD), plus a change subscription that delivers a notification after each committed mutation. Access SHALL be governed by the channel's authentication; unauthorized peers are denied.

#### Scenario: Snapshot reflects committed mutation

- **WHEN** a client performs an alias mutation and then requests a providers snapshot
- **THEN** the snapshot contains the new alias

#### Scenario: Subscribers are notified after commit

- **WHEN** a provider mutation commits
- **THEN** subscribed clients receive a change notification scoped to providers

#### Scenario: Unauthorized peer denied

- **WHEN** an unauthenticated peer calls a state method
- **THEN** the request is denied with an authorization error

### Requirement: Mutation durability

Every state mutation SHALL be committed to the database before its method response is returned. There is no batching, debounce, or shutdown-only flush. Committed mutations SHALL survive an unclean termination of the core process.

#### Scenario: Mutation survives SIGKILL

- **WHEN** a provider mutation's response has returned and core is immediately killed
- **THEN** after restart the provider snapshot includes the mutation

### Requirement: Unavailable store degrades honestly

A client that cannot reach the state store SHALL present an explicit unavailable state naming the cause. It MUST NOT fabricate success, present stale snapshots as current, or silently discard mutations.

#### Scenario: Core unreachable disables state features

- **WHEN** core is not running and a client requests a state snapshot
- **THEN** the client presents an explicit "core 未连接" state with the cause, and mutation entry points are disabled

### Requirement: Corrupt store is not silently reset

A database that cannot be opened due to corruption SHALL block the affected startup with a diagnostic naming the file path. The system MUST NOT delete, truncate, or recreate the database automatically in this case. Corruption and schema incompatibility are distinct and MUST NOT be conflated: corruption means the file cannot be opened or read at the SQLite level; schema incompatibility means the file opens but its structure diverges from the models, and only incompatibility may trigger the automatic reset. Manual recovery of a corrupt database (restoring the user's own copy or deleting it by hand) is outside the system's responsibilities.

#### Scenario: Corrupt database aborts startup with diagnostic

- **WHEN** the database file is corrupt and core starts
- **THEN** startup aborts with an error naming the path, and the file is left untouched

#### Scenario: No silent reset across restarts

- **WHEN** the corrupt database persists across restart attempts
- **THEN** every attempt fails with the same diagnostic and user data is never automatically discarded

#### Scenario: Reset never fires for an unopenable file

- **WHEN** a schema check would run against a database that fails to open
- **THEN** the startup aborts with the corruption diagnostic instead of resetting, even if the structure would also have been judged incompatible

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

### Requirement: Schema self-description and startup sync

The schema SHALL be derived from the code's model objects: each registered table's column set is declared by its model struct, which is the single source of truth. On open, the store SHALL stamp self-describing version metadata as key-value pairs (`version_format` with value `date`, and `version` with a date constant that is bumped when the schema changes — never a wall-clock read) so any database file can be diagnosed as to which schema date produced it. The store SHALL then reconcile each registered table against the live database: missing columns SHALL be added in place (`ALTER TABLE ADD COLUMN`) with their constant default; a structure that cannot be reconciled — a type mismatch, an extra column, a missing table, or an absent/unknown version format — SHALL reset the database: delete the file and rebuild an empty schema from the current models, logging honestly that a schema incompatibility triggered the reset. The version value alone SHALL NOT trigger a reset; structure comparison is the only reset trigger. A reset MUST NOT run for a database that cannot be opened at all — that is corruption, governed by the corrupt-store requirement.

#### Scenario: Fresh database is created from current models

- **WHEN** core starts with no existing database
- **THEN** the schema is created from the current model definitions and the version metadata is stamped (`version_format=date`, `version=<SCHEMA_VERSION>`)

#### Scenario: Missing column is added in place

- **WHEN** a model struct gains a column and the database lacks it
- **THEN** startup adds the column via `ALTER TABLE ADD COLUMN` with its constant default, existing rows remain readable, and no other table is touched

#### Scenario: Incompatible structure resets the database

- **WHEN** a table's live structure diverges irreconcilably from the model (type mismatch, extra column, or the table is absent entirely)
- **THEN** the database is deleted and rebuilt as an empty schema from the current models, and the log names the incompatibility that caused the reset

#### Scenario: Unknown version format resets the database

- **WHEN** the database lacks the version metadata or carries an unrecognized `version_format` (including databases produced by the retired migration chain)
- **THEN** the database is reset to the current schema with the reset recorded in the log

#### Scenario: Version value alone never resets

- **WHEN** the stamped `version` differs from the binary's constant but every registered table's structure matches the models exactly
- **THEN** no reset occurs; the version metadata is updated to the current value
