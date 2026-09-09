## Why

`session-persistence`（39 行，仅 2 条 requirement）的 Purpose 自述「persistence responsibility is being migrated to the core state store (SQLite)」——迁移早已由 `state-store` capability 完成，但旧 capability 仍挂在 spec 树上，与 `state-store`/`provider-management` 构成「时间性重叠」却没有退役时限。两条 requirement 分属不同 domain（默认选择语义属 provider 行为、运行时状态边界属持久化载体），应各归其位后让 `session-persistence` 退役。

## What Changes

- **`session-persistence`「Default selection semantics」→ provider-management**：默认选择（provider + optional model）的语义由 provider-management 的「Set default provider and model from the page」延续；本 change 将 object/bare-string wire 形式兼容、原子删除清默认的语义补入 provider-management，并注明持久化载体已迁移至 state store（SQLite）。
- **`session-persistence`「Runtime state is not persisted by this store」→ state-store**：权限 allowlist、未决卡片、in-flight spawn 不持久化 + 会话 map 每变更持久（非仅关停写入）的语义补入 state-store。
- **`session-persistence` capability 退役**：目录从 `openspec/specs/` 移除，REMOVED 记录随 retire change 归档。
- `glossary.md`「易混对照」如引用 session-persistence 一并清理。

## Capabilities

### New Capabilities

### Modified Capabilities
- `provider-management`: 默认选择 wire 兼容与原子删除语义并入
- `state-store`: 运行时状态边界（不持久化清单 + 会话 map 每变更持久）并入

## Impact

- `openspec/specs/session-persistence/spec.md` 删除（git 历史保留），`changes/archive/` 增加退役记录。
- testsuite-acceptance 核心集如列 session-persistence 一并调整。
- **Non-goals**：不改任何源码；不引入新的默认选择数据模型（沿用 provider store / state store 现状）；不处理 acp-session-mapping 与会话 map 的关系（另域）。
