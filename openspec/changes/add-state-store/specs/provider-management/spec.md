## REMOVED Requirements

### Requirement: Broken overlay self-heal
**Reason**: overlay 文件不复存在;SQLite 的完整性保证使"静默降级到空 overlay 重建"变成数据风险。损坏策略改为显式失败、保留现场、绝不自动重置(state-store "Corrupt store is not silently reset");存储不可用时按 state-store "Unavailable store degrades honestly" 呈现。
**Migration**: 迁移备份(`sebas.db.backup-*`)是人工恢复路径;开发阶段亦可直接弃库重建。卡片侧行为由新增的 "Provider card reflects store availability" 承接。

## ADDED Requirements

### Requirement: Provider card reflects store availability

The `/provider` card SHALL render normally while the state store is reachable. When the store is unavailable or corrupt, the card SHALL present an explicit unavailable state with the cause, disable mutation entry points, and leave all user data untouched.

#### Scenario: Store unavailable shows cause

- **WHEN** the state store is unreachable while a `/provider` card flow is active
- **THEN** the card renders an explicit unavailable state naming the cause, with mutations disabled

#### Scenario: No silent data loss from the card path

- **WHEN** the state store reports corruption
- **THEN** no card-driven operation deletes or resets provider data; recovery goes through the documented manual paths
