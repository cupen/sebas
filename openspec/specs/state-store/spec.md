## Purpose

Owns the SQLite-backed domain state of sebas: where the database lives, how schema versioning and automatic migration behave, who may write, which state methods the core channel exposes, and the durability and unavailability contracts that replace the former per-file JSON semantics.

## ADDED Requirements

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

### Requirement: Schema version and auto-migration

The database SHALL carry an integer schema version. On open, core SHALL apply pending migrations in ascending order, each migration entirely within one transaction together with its version bump; no separate migration step or operator action is required. Migration failure SHALL abort the affected startup and leave the database at its previous version. A database whose version is newer than this binary knows SHALL be refused with a diagnostic naming the version — it MUST NOT be silently read or rewritten. There is no migration of legacy JSON data: the first migration SHALL create the schema and the store starts empty.

#### Scenario: Fresh database is created at current version

- **WHEN** core starts with no existing database
- **THEN** the database is created and stamped with the current schema version

#### Scenario: Pending migrations apply transactionally

- **WHEN** a database at version N is opened by a binary knowing migrations through N+2
- **THEN** migrations apply in order and the stamped version ends at N+2

#### Scenario: Failed migration rolls back

- **WHEN** a migration fails partway through its transaction
- **THEN** the database remains at its previous version with content unchanged, and startup aborts with the error

#### Scenario: Newer database is refused

- **WHEN** a binary that knows schema version N opens a database stamped N+1
- **THEN** startup aborts with a diagnostic naming the version, and the file is not modified

### Requirement: Pre-migration backup

Before applying any migration, the system SHALL produce a consistent snapshot backup of the database at a sibling path recording the source version, and SHALL retain the most recent backup.

#### Scenario: Backup exists after migration

- **WHEN** a migration completes
- **THEN** a snapshot taken at the source version exists next to the database

#### Scenario: Backup usable for manual recovery

- **WHEN** the user restores the retained backup over the database file
- **THEN** the database opens again at the source version

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

A database that cannot be opened due to corruption SHALL block the affected startup with a diagnostic naming the file path. The system MUST NOT delete, truncate, or recreate the database automatically; the retained pre-migration backup remains the manual recovery path.

#### Scenario: Corrupt database aborts startup with diagnostic

- **WHEN** the database file is corrupt and core starts
- **THEN** startup aborts with an error naming the path, and the file is left untouched

#### Scenario: No silent reset across restarts

- **WHEN** the corrupt database persists across restart attempts
- **THEN** every attempt fails with the same diagnostic and user data is never automatically discarded
