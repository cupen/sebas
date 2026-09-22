## MODIFIED Requirements

### Requirement: Schema self-description and startup sync

The schema SHALL be derived from the code's model objects: each registered table's column set is declared by its model struct, which is the single source of truth. On open, the store SHALL stamp self-describing version metadata as key-value pairs (`version_format` with value `date`, and `version` with a date constant that is bumped when the schema changes — never a wall-clock read) so any database file can be diagnosed as to which schema date produced it. The store SHALL then reconcile each registered table against the live database: missing columns SHALL be added in place (`ALTER TABLE ADD COLUMN`) with their constant default; a structure that cannot be reconciled — a type mismatch, an extra column, a missing table, or an absent/unknown version format — SHALL rebuild the schema as empty from the current models, and SHALL preserve the pre-existing file first rather than destroying it: the file and its write-ahead sidecars SHALL be renamed to sibling paths carrying the reset timestamp, and the log SHALL name both the incompatibility that triggered the reset and the quarantine paths, so a reset is reversible by hand. The version value alone SHALL NOT trigger a reset; structure comparison is the only reset trigger. A reset MUST NOT run for a database that cannot be opened at all — that is corruption, governed by the corrupt-store requirement.

#### Scenario: Fresh database is created from current models

- **WHEN** core starts with no existing database
- **THEN** the schema is created from the current model definitions and the version metadata is stamped (`version_format=date`, `version=<SCHEMA_VERSION>`)

#### Scenario: Missing column is added in place

- **WHEN** a model struct gains a column and the database lacks it
- **THEN** startup adds the column via `ALTER TABLE ADD COLUMN` with its constant default, existing rows remain readable, and no other table is touched

#### Scenario: Incompatible structure resets the database

- **WHEN** a table's live structure diverges irreconcilably from the model (type mismatch, extra column, or the table is absent entirely)
- **THEN** the pre-existing database file and its write-ahead sidecars are renamed to sibling paths carrying the reset timestamp
- **AND** an empty schema is rebuilt from the current models
- **AND** the log names the incompatibility that caused the reset and the quarantine paths, so the previous contents can be recovered by hand

#### Scenario: Unknown version format resets the database

- **WHEN** the database lacks the version metadata or carries an unrecognized `version_format` (including databases produced by the retired migration chain)
- **THEN** the database is reset to the current schema with the reset recorded in the log
- **AND** the pre-existing file is quarantined rather than deleted

#### Scenario: Version value alone never resets

- **WHEN** the stamped `version` differs from the binary's constant but every registered table's structure matches the models exactly
- **THEN** no reset occurs; the version metadata is updated to the current value

#### Scenario: A quarantined database is recoverable

- **WHEN** a reset has occurred because of a schema incompatibility
- **THEN** the quarantined file is still a valid SQLite database holding the state as it was before the reset
- **AND** it can be opened by hand to recover values that were not re-created
