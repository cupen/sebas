## REMOVED Requirements

### Requirement: State file layout and overrides
**Reason**: 持久化载体从 state.json/providers.json 双文件改为 state-store 能力的 SQLite 数据库;路径与环境变量覆盖语义由 state-store 的 "Database location and single-writer ownership" 接管。
**Migration**: 无数据迁移——开发阶段旧 JSON 数据直接放弃,新库从零开始。

### Requirement: Current schema version
**Reason**: schema 版本与迁移职责整体移交 state-store 的自动迁移框架(user_version + 顺序迁移链)。
**Migration**: 见 state-store "Schema version and auto-migration"。

### Requirement: Migration from v1
**Reason**: 文件版 v1→v2 迁移不复存在;v1 文件不再被读取。
**Migration**: 无——开发阶段旧文件数据直接放弃,新库从零开始。

### Requirement: One-time overlay reconciliation
**Reason**: overlay 文件消失后不再有文件间调和。
**Migration**: 无——调和语义随 JSON 文件一起退役。

### Requirement: Legacy field upgrade within v2
**Reason**: legacy 字段处理随 JSON 文件一起退役,不再是存储层需求。
**Migration**: 无——开发阶段旧文件数据直接放弃。

### Requirement: Corruption tolerance
**Reason**: JSON 时代"降级到默认状态"的策略不适用于数据更完整的数据库:损坏策略改为显式失败、保留现场、绝不自动重置。
**Migration**: 见 state-store "Corrupt store is not silently reset"。

### Requirement: Atomic write per mutation
**Reason**: 临时文件 + rename 的原子写被单事务提交取代;"变更即持久、无关机刷写"的契约由 state-store 的 "Mutation durability" 承接。
**Migration**: 见 state-store "Mutation durability"。

## MODIFIED Requirements

### Requirement: Default selection semantics

The default selection SHALL comprise a provider name and an optional model; the model SHALL be stored as absent when unset. The channel wire format for the default selection SHALL accept both the object form and the legacy bare-string form. Deleting a provider SHALL atomically (in one transaction) remove its entry, record the deletion, and clear a default selection that names it; mode cleanup is applied by the load-time repair step.

#### Scenario: No model omits the field

- **WHEN** the default selection has a provider but no model
- **THEN** the stored state has no model value, and the wire form omits the model field

#### Scenario: Deleting the default provider clears the selection atomically

- **WHEN** the user deletes the provider that the default selection names
- **THEN** a single transaction removes the entry, records the deletion, and clears the selection

### Requirement: Runtime state is not persisted by this store

The store SHALL NOT persist the permission allowlist, outstanding permission cards, card states, or in-flight spawn placeholders. The agent session map SHALL be persisted in the state store and written per mutation, rather than only at daemon shutdown.

#### Scenario: Allowlist survives no restart

- **WHEN** the daemon restarts
- **THEN** previously granted session-scoped permissions are gone and the user is prompted again

#### Scenario: Spawn placeholders are never written

- **WHEN** the daemon shuts down while a spawn is in flight
- **THEN** the persisted state contains no trace of the in-flight spawn

#### Scenario: Session map survives unclean exit

- **WHEN** core is killed while sessions are active
- **THEN** the session map at next start reflects the last committed state, not the last graceful shutdown
