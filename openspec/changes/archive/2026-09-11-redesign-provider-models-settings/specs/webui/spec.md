## MODIFIED Requirements

### Requirement: Provider management page

The WebUI settings SHALL provide a provider management page backed by the core-owned
provider store (see `provider-management`), reached through the WebUI's
`/router/api/providers*` surface. It SHALL support: listing providers with name,
preset-or-custom mark, base URL slots, key-configured state, and each model entry's
capability tags; creating a provider either from a preset or as custom; editing an
existing provider; and deleting a provider. Fetching a provider's official model list is
specified separately by the `Fetch models from the provider's official base URL`
requirement, not here.

The model list SHALL be a list of entries the operator may add to and remove from freely,
each entry carrying an id and its capability tags (`text` implicit, with `vision`,
`audio`, and `video` selectable). The list SHALL be editable for preset-derived and custom
providers alike.

Creating from a preset SHALL require only the preset choice, the API key, and optionally
the model entries; the preset's base URLs SHALL render as read-only code-owned values.
Creating a custom provider SHALL require the same inputs plus an instance name, a base URL,
and a wire protocol. Every input beyond that minimum — the remaining base URL slots, the
model rename map, and the preset's inherited `api_key_env` — SHALL live in an Advanced
disclosure that is collapsed by default. `api_key_env` SHALL NOT be a user input field; a
preset's env name remains only a fallback key source when no plaintext key is stored.

Secret inputs SHALL never be pre-filled, and an empty secret submit SHALL preserve the
stored key. Mutations SHALL surface their errors (duplicate, validation, unavailable) in
the page. Choosing an option inside any select control of the provider editor SHALL NOT
close the editor.

#### Scenario: preset-derived provider shows read-only URLs

- **WHEN** the user opens provider `glm` (preset-derived) for editing
- **THEN** the base URL fields render the preset's code-owned values as read-only with a
  follow-the-code indication, while the API key, default model, model entries, and their
  capability tags remain editable

#### Scenario: create from preset

- **WHEN** the user creates a provider from preset `glm` entering an API key and two model
  entries
- **THEN** the create request carries the preset selection, the key, and the model entries
  with capability tags, but no base URL fields, and the provider appears in the list

#### Scenario: custom provider full editing

- **WHEN** the user creates a custom provider filling a single base URL
- **THEN** the create succeeds and the edit form exposes all three slots — the two extras
  under the Advanced disclosure — for later adjustment

#### Scenario: probe from the page

- **WHEN** the user runs a fetch on a provider with a usable base URL
- **THEN** the page shows the returned model ids without leaving the page

#### Scenario: delete reflects immediately

- **WHEN** the user deletes provider `alpha` from the page
- **THEN** the provider disappears from the list without a page reload

#### Scenario: preset instance name defaults without input

- **WHEN** the user creates from preset `glm` without opening the Advanced disclosure
- **THEN** the provider is stored under the name `glm`, and renaming it is the only path to
  a second instance of the same preset

#### Scenario: custom provider minimal input

- **WHEN** the user creates a custom provider entering an instance name, a base URL, and a
  wire protocol
- **THEN** the create succeeds without the operator opening the remaining URL slots or the
  model rename map

#### Scenario: advanced section collapsed by default

- **WHEN** the provider editor opens in either mode
- **THEN** the remaining base URL slots, the model rename map, and the inherited
  `api_key_env` are behind a collapsed disclosure, and none of them is submitted as a
  user-entered value unless the operator opens it

#### Scenario: multiple model entries with capability tags

- **WHEN** the user adds several model entries to a provider and marks one as `vision`
- **THEN** the list preserves order, that entry carries `vision` alongside the implicit
  `text`, and the others carry `text` alone

#### Scenario: model list is editable for a preset-derived provider

- **WHEN** the user adds a model entry to a preset-derived provider
- **THEN** the entry is stored on that provider without altering the preset's code-table
  data

#### Scenario: selecting an option does not close the editor

- **WHEN** the user picks a preset, a protocol, or a model option inside the provider
  editor
- **THEN** the editor stays open and the chosen value is retained

#### Scenario: secret is preserved on empty submit

- **WHEN** the user edits a provider and submits with the API key field left empty
- **THEN** the stored key is unchanged and no key material was rendered

## REMOVED Requirements

### Requirement: Services 分区数据源

**Reason**: 该需求原先要求「Models 分区顶部呈现 provider 路由网关总览（listen /
debug / auth）」，把 router 运行状态放进 provider 管理面。本 change 反转这一归属：
router 状态只在 Services 分区呈现，Models 分区只保留 provider 管理。原需求的场景
「Models 分区承载 router 总览」与新契约直接冲突，无法在 MODIFIED 中保留（工具链
不允许 MODIFIED 丢弃场景名），故整体退役并以新需求取代。

**Migration**: 由 `Services 分区与 router 状态归属` 取代，受管服务渲染、无
watchdog 退化、enable/disable/restart 场景全部继承，另增「router 状态只在
Services 呈现」。

## ADDED Requirements

### Requirement: Services 分区与 router 状态归属
Services 分区 SHALL 以 watchdog 受管子进程为唯一数据源：调用 `GET /api/admin/services` 获取受管服务表，渲染每个进程的 name / desired / actual status / uptime_secs / 最近错误（由 `/api/admin/events` 提供，无事件则不渲染错误行）。受管服务名固定为 `core` / `webui` / `router` / `im`（IM 在配置未启用时不出现；产品对外名称保留「飞书」由前端做 i18n）。core 为恒启动服务：其行 SHALL 仅呈现状态与 restart 入口，SHALL NOT 渲染 enable/disable 按钮（enable-core-by-default）。无 watchdog adapter 时 SHALL 显式呈现 `adapter_ok: false` 横幅、不暴露 enable/disable/restart 按钮；该形态下 `/api/admin/services` 返回空数组且后端响应携带 `adapter_ok: false`。router 的运行状态（desired / actual / uptime）SHALL 仅由本分区呈现；Models 分区 SHALL NOT 呈现 router 网关总览、listen / debug / auth 或任何 router 运行状态，provider 管理面与 router 运行状态在产品语义上分离。

#### Scenario: Services 渲染受管子进程

- **WHEN** watchdog 拉起 core/webui/router/im 四个进程，Services 分区聚焦
- **THEN** 列表呈现 core / webui / router / im 四行及 desired / actual / uptime；im 在配置未启用时不出现该行

#### Scenario: 无 watchdog adapter 退化

- **WHEN** webui 在裸 core 形态下启动（无 SEBAS_CONTROL_SECRET）
- **THEN** Services 分区显示「无 watchdog 控制面」横幅、列表为空、enable/disable/restart 按钮不渲染

#### Scenario: enable 成功

- **WHEN** 操作员对 `router` 点击 enable
- **THEN** 前端 POST `/api/admin/services/router/enable` 收到 200；列表行刷新（desired 变 on）；若后端返回 503 则内联呈现错误且不刷新

#### Scenario: disable 成功

- **WHEN** 操作员对辅助服务（如 `router`）点击 disable（前提：confirm 弹窗已确认）
- **THEN** 前端 POST `/api/admin/services/router/disable` 收到 200；列表行刷新；core 行不渲染 enable/disable 按钮

#### Scenario: restart 操作

- **WHEN** 操作员对 `core` 点击 restart
- **THEN** 前端走 `Admin actions via control plane` 既有 restart-core 路径；成功后列表行 uptime 重置

#### Scenario: router 状态只在 Services 呈现

- **WHEN** 操作员分别聚焦 Models 与 Services 分区
- **THEN** Models 分区只呈现 provider 列表与管理入口，不渲染 router 的 listen / debug / auth 或可达性；router 的 desired / actual / uptime 只在 Services 分区呈现，且该分区在无 watchdog adapter 时如实说明不可用
