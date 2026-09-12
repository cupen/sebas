## REMOVED Requirements

### Requirement: Schema version and auto-migration

**Reason**: 项目未正式发布、无存量数据需要保全，逐级迁移链（`MIGRATIONS` 数组、版本推进、过高版本拒启）让每一次"model struct 加个列"都要人工写迁移脚本。开发期的破坏性 schema 修改改由启动时与代码内 model 对比自动同步承接，代码即 schema 的单一事实源。

**Migration**: 无。旧库（带 `user_version`、无自描述版本键）在升级后的首次打开按"未知版本格式"重置为当前 model 的空 schema，日志如实记录；无需任何数据保全动作。

### Requirement: Pre-migration backup

**Reason**: 备份的唯一用途是迁移失败的回滚逃生舱；迁移链退役后该场景不复存在，"不兼容即重置"的开发期语义明确不为重置保留备份（开发库可弃，确需保留时手动拷文件）。

**Migration**: 无。存量备份文件不受影响（无人清理也无人再生成）；恢复路径由使用者自行掌握。

## ADDED Requirements

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

## MODIFIED Requirements

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
