## MODIFIED Requirements

### Requirement: Execution-body availability is stated, not discovered

The composer's execution-body selector SHALL reflect, for each execution body,
whether it can serve new sessions in the current process configuration. An
execution body that cannot serve new sessions — for example the native kernel
running without provider credentials — SHALL be presented as unavailable with
its cause stated, and SHALL NOT be selectable such that the operator only
discovers the failure on submission. Availability SHALL be derived from the
session backend's own report of both execution bodies, not from the ACP side
alone.

The cause SHALL be stated in operator-facing language and SHALL name the
remediation surface that can actually clear it：

- 对 WebUI 内可补救的不可用（如 agent 启动定义缺失 → 「设置 → Agent」；provider
  模型缺失 → 「设置 → 模型」），cause SHALL 指向该设置面，且 SHALL NOT 在默认
  可见文案里暴露内部环境变量名等实现标识（此类标识 MAY 出现在 tooltip）。
- 对 native 内核：其启用条件是 core 进程启动 env
  （`SEBAS_AGENT_PROVIDER_API_KEY` 或 `SEBAS_AGENT_ROUTER_URL`），WebUI 内
  **无法**补救——cause SHALL 如实陈述该 env 条件（默认可见文案中点名环境变量
  标识符对此情形是**要求**而非违规），SHALL NOT 引导操作者到对启用无效的
  WebUI 设置面（QA round13 实证：在「设置 → 模型」配置 provider 不能点亮
  native）。

#### Scenario: native kernel without credentials shown as unavailable

- **WHEN** the native kernel has no provider credentials and the composer is
  rendered
- **THEN** the `native` option is shown as unavailable with the cause stated,
  and submitting a native spawn is prevented at the composer rather than
  failing at the core

#### Scenario: unavailable cause speaks operator language

- **WHEN** an execution body is unavailable for a cause that is remediable
  inside the WebUI（如启动定义缺失）
- **THEN** the visible cause names the remediation settings surface and
  contains no environment variable identifier

#### Scenario: native cause names the core env condition

- **WHEN** the native kernel is unavailable because the core process was
  started without `SEBAS_AGENT_PROVIDER_API_KEY` and without
  `SEBAS_AGENT_ROUTER_URL`
- **THEN** the visible cause names at least one of those environment variable
  identifiers and states that it cannot be configured inside the WebUI
- **AND** the cause does not direct the operator to a settings surface（如
  「设置 → 模型」）as a way to enable native

#### Scenario: both bodies available

- **WHEN** both the ACP bridge and the native kernel can serve new sessions
- **THEN** the selector offers both without degradation notices

#### Scenario: availability recovers without reload

- **WHEN** the cause making an execution body unavailable is resolved while the
  page stays open
- **THEN** the selector offers that body again without the operator reloading
