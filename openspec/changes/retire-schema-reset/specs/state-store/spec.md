## REMOVED Requirements

### Requirement: Schema self-description and startup sync

**Reason**: 该需求的核心语义是「结构不符即删库重建」（类型不符 / 多余列 / 缺表 / 未知版本格式一律删文件重建空 schema）。项目进入真实使用后 providers、projects、session_map 里的状态数据有保全价值，且 SQLite ≥ 3.50 已支持 `RENAME COLUMN` / `DROP COLUMN` / 事务内 DDL 重建——重置不再有存在理由。行为核心被整体反转，故退役整个需求而非逐场景修改。

**Migration**: 由本变更的 ADDED 需求「Schema self-description and non-destructive sync」（同步契约）与「Destructive schema migration is backed up and fails closed」（备份与 fail-closed 语义）承接。存量开发库升级后首开被结构 reconcile 吸纳（结构一致仅补版本键，不一致走保数据迁移），任何路径都不再删除数据库文件。

## ADDED Requirements

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

The sync path SHALL never delete the database file. Before performing any destructive migration step (a table rebuild or a column drop), the store SHALL write a backup copy of the whole database adjacent to the database file, replacing any previous backup; if the backup cannot be written, the destructive migration SHALL NOT run and startup SHALL abort with a diagnostic naming the backup failure. Every migration SHALL run inside a transaction; if a migration step fails, the transaction SHALL roll back, the database SHALL be left byte-identical to before the attempt, and startup SHALL abort with a diagnostic naming the failed step — the store MUST NOT fall back to resetting or rebuilding the database as a side effect of a failed migration.

#### Scenario: Backup precedes a destructive migration

- **WHEN** a type change or column drop is about to run
- **THEN** a backup copy of the database is written next to the database file first, and the migration proceeds only after the backup succeeds

#### Scenario: Backup failure blocks the destructive migration

- **WHEN** the backup copy cannot be written
- **THEN** the destructive migration does not run, the database is left untouched, and startup aborts with a diagnostic naming the backup failure

#### Scenario: Failed migration rolls back and refuses startup

- **WHEN** a migration step fails mid-reconciliation (for example, the data copy errors)
- **THEN** the transaction rolls back leaving the database unchanged, startup aborts with a diagnostic naming the failed step, and no reset or file deletion occurs
