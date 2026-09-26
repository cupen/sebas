# state-store Specification

## Purpose

Owns the SQLite-backed domain state of sebas: where the database lives, how schema versioning and automatic migration behave, who may write, which state methods the core channel exposes, and the durability and unavailability contracts that replace the former per-file JSON semantics.

## Requirements

### Requirement: Database location and single-writer ownership

The domain state SHALL live in a set of purpose-layered SQLite databases inside a single state directory, each opened in WAL mode, rather than in one undifferentiated database. Layering SHALL follow a two-level rule: first by **writing process** — each database SHALL have exactly one writer, and a process that is not a database's writer SHALL NOT open it, accessing that state exclusively through the core channel state methods; then, within the core's own databases, by **growth characteristic** — bounded system configuration (providers, model aliases, card and runtime settings, whose row count is decided by hand-written configuration) SHALL be separated from user data that grows with use (projects, the session map, and the session and message content that follows it). Paths SHALL expand a leading `~/`. All mutations SHALL be applied by the owning process's state store, serialized one at a time per database. Because the databases are layered, an operation that rebuilds or resets one of them SHALL NOT affect the other.

#### Scenario: Environment override relocates the database

- **WHEN** the environment variable for one database points to a custom path
- **THEN** the owning process opens that database at that path
- **AND** the other databases keep resolving inside the state directory

#### Scenario: Tilde paths expand to home

- **WHEN** a configured database path begins with `~/`
- **THEN** the path is expanded to the user's home directory before use

#### Scenario: Concurrent mutations serialize

- **WHEN** two clients issue state mutations concurrently
- **THEN** both apply in serialization order and a later snapshot reflects the combined result — never a torn or lost update without an explicit error

#### Scenario: bounded configuration is separated from growing user data

- **WHEN** the databases are inspected after extended use
- **THEN** provider, model-alias, and settings rows live in the bounded configuration database
- **AND** projects and the session map live in the user-data database
- **AND** the configuration database's size is not driven by how much the user accumulates

#### Scenario: one database per writer

- **WHEN** the set of databases and their writers is inspected
- **THEN** each database is written by exactly one process
- **AND** no database is opened by a process other than its writer

#### Scenario: resetting user data leaves configuration intact

- **WHEN** the user-data database is rebuilt because its structure diverged from the models
- **THEN** the configuration database is untouched
- **AND** settings survive the rebuild

#### Scenario: a database's unavailability is reported, not hidden

- **WHEN** one database cannot be opened while another opens successfully
- **THEN** the unavailable domain reports an explicit unavailable state naming the cause
- **AND** the process does not present substitute or default values as if the state were current

### Requirement: State methods on the core channel

The core channel SHALL expose state methods for snapshot queries (domains `providers` — projected with their model aliases — `settings`, `projects`, `presets`, `sessions`, and `router_activity`) and mutations (provider/alias/settings/projects CRUD; `aliases` is its own mutation domain), plus a change subscription that delivers a notification after each committed mutation. Access SHALL be governed by the channel's authentication; unauthorized peers are denied.

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

### Requirement: Schema self-description and non-destructive sync

The schema SHALL be derived from the code's model objects: each registered table's column set is declared by its model struct, which is the single source of truth. On open, the store SHALL stamp self-describing version metadata as key-value pairs (`version_format` with value `date`, and `version` with a date constant that is bumped when the schema changes — never a wall-clock read) so any database file can be diagnosed as to which schema date produced it; this metadata is diagnostic only and SHALL NOT gate any sync action. The store SHALL then reconcile each registered table against the live database without ever deleting the database file: a missing column SHALL be added in place (`ALTER TABLE ADD COLUMN`) with its constant default; a column the model declares as renamed (an explicit rename annotation on the struct field naming the previous column) SHALL be migrated via `ALTER TABLE RENAME COLUMN` with its data preserved, and an undeclared missing-plus-extra pair SHALL be treated as a drop plus an add, never guessed as a rename; a column type mismatch SHALL be resolved by rebuilding the table inside a single transaction (create the new table from the registered DDL, copy existing rows column-by-name with SQLite affinity coercion, swap the names, recreate the indexes) so existing data is carried over; a column present in the database but absent from the model SHALL be dropped with its data discarded, using the same table rebuild when the column is referenced by an index or constraint; a missing table SHALL be created. Missing or unrecognized version metadata SHALL NOT trigger a reset: the same structural reconciliation runs and the reconciliation is logged honestly (including databases produced by the retired migration chain). No sync outcome SHALL delete or recreate the database file; corruption remains governed by the corrupt-store requirement. The version value alone SHALL NOT trigger any action; structural comparison is the only trigger.

#### Scenario: Fresh database is created from current models

- **WHEN** core starts with no existing database
- **THEN** the schema is created from the current model definitions and the version metadata is stamped (`version_format=date`, `version=<SCHEMA_VERSION>`)

#### Scenario: Missing column is added in place

- **WHEN** a model struct gains a column and the database lacks it
- **THEN** startup adds the column via `ALTER TABLE ADD COLUMN` with its constant default, existing rows remain readable, and no other table is touched

#### Scenario: Declared rename preserves the column's data

- **WHEN** a model field is renamed and annotated with the previous column name, and the database still carries the old column
- **THEN** startup renames the column in place, every existing row keeps its value under the new column, and no table rebuild occurs

#### Scenario: Undeclared missing-plus-extra pair is not guessed as a rename

- **WHEN** the model lacks a column the database has, and declares a column the database lacks, with no rename annotation connecting them
- **THEN** the extra column is dropped and the missing column is added, and the log names both columns so the divergence is visible

#### Scenario: Type change rebuilds the table without losing rows

- **WHEN** a model field's column type diverges from the live column's affinity
- **THEN** the table is rebuilt inside a single transaction, every existing row is copied into the rebuilt table by column name with SQLite affinity coercion applied, indexes are recreated, and the log names the table and the type mismatch

#### Scenario: Column removed from the model is dropped

- **WHEN** a model field is removed and the database still carries the column, whether or not an index or constraint references it
- **THEN** the column (and only that column's data) is discarded, the rest of the table's rows survive, and the log names the dropped column

#### Scenario: Unknown version metadata reconciles by structure

- **WHEN** the database lacks the version metadata or carries an unrecognized `version_format` (including databases produced by the retired migration chain)
- **THEN** the same structural reconciliation runs without any reset, the database is brought in line with the current models, and the log records that unversioned or unknown-format metadata was reconciled

#### Scenario: Version value alone never triggers action

- **WHEN** the stamped `version` differs from the binary's constant but every registered table's structure matches the models exactly
- **THEN** no migration or rebuild occurs; the version metadata is updated to the current value

### Requirement: Destructive schema migration is backed up and fails closed

The sync path SHALL never delete the database file. Before performing any destructive migration step (a table rebuild or a column drop), the store SHALL write a backup copy of the whole database adjacent to the database file, replacing any previous backup; the backup copy SHALL be tightened to the same owner-only permissions as the database file, because it is a complete copy of the same content (including provider credentials); if the backup cannot be written, the destructive migration SHALL NOT run and startup SHALL abort with a diagnostic naming the backup failure. Every migration SHALL run inside a transaction; if a migration step fails, the transaction SHALL roll back, the database SHALL be left byte-identical to before the attempt, and startup SHALL abort with a diagnostic naming the failed step — the store MUST NOT fall back to resetting or rebuilding the database as a side effect of a failed migration.

#### Scenario: Backup precedes a destructive migration

- **WHEN** a type change or column drop is about to run
- **THEN** a backup copy of the database is written next to the database file first, and the migration proceeds only after the backup succeeds

#### Scenario: The backup copy is not more permissive than the database

- **WHEN** the backup copy has been written
- **THEN** its file permissions are owner-only, matching the database file, so the copy does not re-expose content the database itself protects

#### Scenario: Backup failure blocks the destructive migration

- **WHEN** the backup copy cannot be written
- **THEN** the destructive migration does not run, the database is left untouched, and startup aborts with a diagnostic naming the backup failure

#### Scenario: Failed migration rolls back and refuses startup

- **WHEN** a migration step fails mid-reconciliation (for example, the data copy errors)
- **THEN** the transaction rolls back leaving the database unchanged, startup aborts with a diagnostic naming the failed step, and no reset or file deletion occurs
