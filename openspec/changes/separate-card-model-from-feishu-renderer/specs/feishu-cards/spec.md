## REMOVED Requirements

### Requirement: Per-turn card model
**Reason**: per-turn 单卡片实例、frozen-at-turn-end、in-flight 不新建卡片的契约是通道中立语义，随卡片模型中立化迁入 channels 的「Neutral presentation content contract」。
**Migration**: 见 channels「Neutral presentation content contract」（per-turn 实例 + frozen + in-flight 不建新）。

### Requirement: Card structure
**Reason**: 卡片结构（header/quote/body/footer）描述的是中立呈现的累积内容布局，其飞书 UI 细节（emoji、footer 文案格式）属 renderer 实现；中立「结构化累积」契约由 channels 承接，布局细节降级为 renderer 专属（见 feishu-cards「Feishu card layout details」）。
**Migration**: 见 channels「Neutral presentation content contract」；飞书布局渲染见 feishu-cards「Feishu card layout details」。

### Requirement: Streaming update cadence
**Reason**: 流式合并（debounce coalesce + terminal 事件即刷）是中立流式呈现契约，迁入 channels「Neutral presentation content contract」。
**Migration**: 见 channels「Neutral presentation content contract」（coalesce + terminal 即刷场景）。

### Requirement: Terminal state rendering
**Reason**: 终态呈现（finish 标记、terminal error 记录）是中立呈现生命周期契约，迁入 channels「Neutral presentation content contract」。
**Migration**: 见 channels「Neutral presentation content contract」（terminal state + error marker 场景）。

### Requirement: Thinking display policy
**Reason**: thinking 折叠/隐藏策略属中立呈现内容契约，迁入 channels「Neutral presentation content contract」；飞书侧的 `💭` emoji 折叠面板 UI 属 renderer 细节。
**Migration**: 见 channels「Neutral presentation content contract」（thinking shown folded or hidden 场景）。

### Requirement: Long-content truncation and suppression
**Reason**: 长内容截断/工具输出抑制是中立呈现的内容预算契约，迁入 channels「Neutral presentation content contract」的 budget 语义；`[card]` 配置键解析与 `（已折叠 N 字）` 文案属 renderer 侧实现。
**Migration**: 见 channels「Neutral presentation content contract」（budget eviction）；配置解析见 feishu-cards「Card theme configuration」，文案见「Feishu card layout details」。

### Requirement: Body budget and card rotation
**Reason**: 内容预算与轮换（evict 最旧元素、高水位 rotate + 续接 note）是中立呈现契约，迁入 channels「Neutral presentation content contract」。
**Migration**: 见 channels「Neutral presentation content contract」（budget eviction and rotation 场景）。

### Requirement: Card lifecycle cleanup
**Reason**: 会话结束时丢弃卡片状态、防 stale 更新是中立呈现生命周期语义，迁入 channels「Neutral presentation content contract」。
**Migration**: 见 channels「Neutral presentation content contract」（presentation state dropped on session end 场景）。

## MODIFIED Requirements

### Requirement: Interactive card JSON

The Feishu adapter SHALL emit the neutral presentation (channels) as Feishu card schema `2.0` JSON using interactive v2 elements. The element vocabulary used by the system SHALL comprise: `hr`, `markdown`, `div` (text), `div` (fields), `button`, `collapsible_panel`, `form`, `select_static`, and `column_set`. Buttons SHALL be expressed as first-class v2 buttons, not V1 action containers.

#### Scenario: v2 button rendering

- **WHEN** a permission card with three decision buttons is rendered
- **THEN** the card JSON contains v2 `button` elements and the card is accepted by the Feishu card API (no V1 `action` block)

#### Scenario: client version constraint

- **WHEN** a card containing `collapsible_panel` is sent
- **THEN** panels render only on Feishu clients at or above the version that introduced collapsible panels

### Requirement: Help card

The `/help` command SHALL render an interactive Feishu help card organized as tabs (命令 / 会话 / 管理 / 通道), with command buttons laid out in 2–3 columns via `column_set` (wide commands taking a full row). Tab switching SHALL update the same card in place (PATCH by message id) rather than sending a new card. Clicking a command button SHALL behave as if the user typed that command's text.

#### Scenario: tab switch in place

- **WHEN** the user clicks the 会话 tab button on the help card
- **THEN** the existing help card is updated in place to show session commands, with no new message in the chat

#### Scenario: command button invocation

- **WHEN** the user clicks a command button on the help card
- **THEN** the router processes the corresponding command text through the same path as a typed message

### Requirement: Error and status cards

System events SHALL be reported as dedicated Feishu cards: spawn failure as a red `❌ 启动失败` card (detail in a code fence when multi-line or over 120 chars); interaction with a dead session as a grey `会话已结束` card; a rejected resume falling back to a fresh session as an orange `已开启新会话` card.

#### Scenario: spawn failure card

- **WHEN** the ACP child fails to start with error `claude not found`
- **THEN** the adapter sends a red card titled `❌ 启动失败` containing the error

#### Scenario: dead session interaction

- **WHEN** a button callback arrives for a session whose mapping is gone
- **THEN** the adapter replies with a grey `会话已结束` card instead of routing the action

### Requirement: Card theme configuration

The Feishu card theme color (`card.theme_color`, default `blue`) SHALL flow into the card header template. Card settings SHALL be parsed with strict (deny-unknown-fields) semantics — an unknown key in `[card]` is a configuration error rather than a silent ignore — and persisted as a full-snapshot JSON file written atomically with mode 0600.

#### Scenario: default theme

- **WHEN** no `[card]` section is configured
- **THEN** card headers render with the blue template

#### Scenario: unknown card key rejected

- **WHEN** the config file contains `[card]` with key `theme_colr`
- **THEN** configuration parsing fails with an unknown-field error

### Requirement: Feishu adapter renders the neutral presentation model

The Feishu adapter SHALL render the neutral outbound presentation model (channels「Neutral outbound presentation」+「Neutral presentation content contract」) into Feishu cards: the adapter SHALL own the mapping from the neutral per-turn content (streaming updates, turn lifecycle, interactive elements) to Feishu card schema 2.0 JSON and to the Feishu send/update/reaction API calls. `CardConfig` rendering knobs (theme, truncation, folding defaults) SHALL be interpreted by the IM service, not by core session routing; the core process SHALL NOT hold card state.

#### Scenario: neutral presentation streams to card JSON

- **WHEN** the IM frontend emits a streaming update for a neutral presentation of a feishu session
- **THEN** the adapter renders it as an `UpdateCard` call to the Feishu card API per the streaming cadence rule in channels「Neutral presentation content contract」

#### Scenario: interactive element maps to feishu callback

- **WHEN** the neutral presentation includes a permission button
- **THEN** the adapter renders a Feishu v2 button whose callback resolves to a neutral button-callback event addressed to the session

#### Scenario: core 重启不影响卡片状态归属

- **WHEN** im 服务持续运行而 core 进程重启
- **THEN** 卡片状态机的归属始终在 im，重启后 im 以新快照重建会话视图，不出现双写卡片状态

### Requirement: Feishu card layout details

Where the neutral presentation content contract leaves layout to the renderer, the Feishu adapter SHALL render the card top-to-bottom as: header (title derived from the first non-empty line of the user prompt, truncated to 40 chars), quote block containing the user prompt, divider, body elements, and footer showing `{model} · in: {input} out: {output} · ctx: {total_input}` when usage is known, otherwise `msg_id: {session_id}`. Emoji affordances (`💭 思考`, `🤔 折腾中`, `✅ 已完成`, `📎 接上条，内容继续`, `❌`) and the truncation note `（已折叠 N 字）` are Feishu-renderer vocabulary rendered on top of the neutral contract's folding/budget markers.

#### Scenario: usage footer

- **WHEN** a turn finishes with a usage event carrying input=10, output=25, total_input=12 for model `claude-x`
- **THEN** the card footer renders `claude-x · in: 10 out: 25 · ctx: 12`

#### Scenario: title truncation

- **WHEN** the user prompt's first non-empty line is 80 characters
- **THEN** the card header title shows only the first 40 characters

#### Scenario: finished panel rename

- **WHEN** a turn transitions to terminal-finished per the neutral content contract
- **THEN** the collapsible working panel's header text becomes `✅ 已完成`

#### Scenario: rotation continuation note

- **WHEN** a turn's content reaches the rotation threshold in the neutral content contract
- **THEN** the new continuation card opens with the `📎 接上条，内容继续` note

#### Scenario: thinking folded with emoji affordance

- **WHEN** thinking is shown per the neutral content contract
- **THEN** the body renders a collapsed `💭 思考` panel holding the thinking text, separate from the output text
