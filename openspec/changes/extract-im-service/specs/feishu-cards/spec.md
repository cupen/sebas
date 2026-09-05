# feishu-cards Specification（delta）

## MODIFIED Requirements

### Requirement: Feishu adapter renders the neutral presentation model

The Feishu adapter SHALL render the neutral outbound presentation model into Feishu cards following the card-structure, streaming, thinking, truncation, budget, rotation, and interactive rules below. The IM service's frontend SHALL produce the neutral presentation (streaming updates, turn lifecycle, interactive elements) from the core's session content; the adapter SHALL own the mapping from that model to Feishu card schema 2.0 JSON and to the Feishu send/update/reaction API calls. `CardConfig` rendering knobs (theme, truncation, folding defaults) SHALL be interpreted by the IM service, not by core session routing; the core process SHALL NOT hold card state.

#### Scenario: neutral presentation streams to card JSON

- **WHEN** the IM frontend emits a streaming update for a neutral presentation of a feishu session
- **THEN** the adapter renders it as an `UpdateCard` call to the Feishu card API per the streaming cadence rule below

#### Scenario: interactive element maps to feishu callback

- **WHEN** the neutral presentation includes a permission button
- **THEN** the adapter renders a Feishu v2 button whose callback resolves to a neutral button-callback event addressed to the session

#### Scenario: core 重启不影响卡片状态归属

- **WHEN** im 服务持续运行而 core 进程重启
- **THEN** 卡片状态机的归属始终在 im，重启后 im 以新快照重建会话视图，不出现双写卡片状态
