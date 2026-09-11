## MODIFIED Requirements

### Requirement: Configuration source

The router SHALL read provider overrides (providers, deletions, model aliases) through
the core channel state methods, backed by the core state store, merged on top of the
config seed. The core state store SHALL be the single source of truth for provider and
model data, and core SHALL be its only writer: the feishu `/provider` card path, the
WebUI provider surface, and this process's admin read views all obtain and mutate that
data through core. The router SHALL NOT write provider, alias, or default data by any
path — no admin mutation endpoint, no file fallback, no direct write to
`providers.json` or a defaults file. When no stored provider data exists, the config seed
alone applies. Legacy JSON files SHALL NOT be imported and SHALL NOT be written: the
state store starts empty and remains the only authority.

#### Scenario: card-edited provider reaches router

- **WHEN** the `/provider` card flow stores provider `beta` with an OpenAI base URL and the router loads (or receives a change notification)
- **THEN** `beta` is routable through the router without editing config.toml

#### Scenario: seeded-only machine unchanged

- **WHEN** the state store contains no provider rows
- **THEN** the config seed alone applies

#### Scenario: migrated state file data moves back

- **WHEN** a machine upgrades with providers stored only in the legacy JSON files (state.json / providers.json)
- **THEN** the state store starts empty (no legacy import), and providers are re-created through the `/provider` card or the WebUI provider surface before they route again

#### Scenario: router has no provider write path

- **WHEN** a client sends a POST, PUT, or DELETE against the router's provider, model-alias, or defaults admin routes
- **THEN** no such route exists (404), the store is unchanged, and no provider or defaults file is written

#### Scenario: router without a reachable core writes nothing

- **WHEN** the router runs with no core reachable and a management operation is attempted against it
- **THEN** the operation fails with an unavailable-source error and the on-disk provider data is left untouched

## REMOVED Requirements

### Requirement: Provider CRUD endpoints

**Reason**: provider 的创建、改名、删除是写操作，按新契约只能由 core 执行；router 不再是 provider 数据的管理者，只做只读消费。保留该变更面等于保留 router 的写权威。

**Migration**: 改用 core 状态库通道的 `providers` 域（`StateMutation { domain: "providers", payload }`）；WebUI 后端与飞书卡片都经该域管理 provider。

### Requirement: Model alias CRUD endpoints

**Reason**: 模型别名与 provider 数据同源，写操作同样归 core；router 侧不再提供别名的创建、更新、删除。

**Migration**: 改用 core 状态库通道的 `aliases` 域；别名的路由语义不变，见 router-model-aliases。

### Requirement: Model probe endpoint

**Reason**: 该端点带 `?apply=true` 写回 provider 的模型列表，属于写路径；同时它把上游抓取能力放在 router 上，与新定位相悖。

**Migration**: 上游模型列表抓取改由 core 承载，见 `add-fetch-models`；router 侧不再提供 probe 端点。

### Requirement: Write-then-apply semantics

**Reason**: 该需求描述的是 admin API 自己执行写入并保证事务性；router 不再执行任何 provider 写入，此语义随之失去主体。

**Migration**: 写入的事务性与「先落盘再生效」由 core 承担，见 core-session-channel 的 state store 通道面与 provider-management 的「Core owns provider and model data」；router 侧「变更后无重启生效」由既有 `External change hot reload` 覆盖。
