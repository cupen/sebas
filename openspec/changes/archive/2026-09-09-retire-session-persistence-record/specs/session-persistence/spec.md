## REMOVED Requirements

### Requirement: Default selection semantics
**Reason**: 默认选择的持久化已从 state.json/providers.json 迁移到 state store（SQLite），语义随 provider 行为域并入 provider-management 的「Set default provider and model from the page」与新增「Default selection wire compatibility and atomic delete」。
**Migration**: 见 provider-management「Set default provider and model from the page」与「Default selection wire compatibility and atomic delete」。

### Requirement: Runtime state is not persisted by this store
**Reason**: 存储层迁移完成后，「不持久化清单 + 会话 map 每变更持久」契约由 state-store 承接；store 的载体、迁移与损坏策略见 state-store。
**Migration**: 见 state-store「Runtime state boundaries for persisted session state」（并关联「State methods on the core channel」「Mutation durability」）。
