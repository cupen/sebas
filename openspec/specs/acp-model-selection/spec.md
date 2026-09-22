# acp-model-selection Specification

## Purpose
Lets users pick the model an ACP-backed session runs on. The agent exposes its model choices over standard ACP `configOptions` and accepts switches via `session/set_config_option`; sebas surfaces that list as a per-session model dropdown and forwards the user's choice to the child process. Works for any native ACP agent that advertises a model config option (opencode, gemini, …), with no agent-specific code.

## Requirements

### Requirement: Session model list is exposed

For each ACP session, the system SHALL expose the set of selectable models reported by the agent via its `configOptions` (the `model`-category option's select values), together with the session's current model. The source for generic ACP agents SHALL be the agent's own `session/new`/`session/load` response — never a hardcoded list. The Claude-specific driver — whose control protocol has no config-options channel — SHALL instead expose a built-in alias table (the Claude model families it serves, with an explicit "default" entry), overridable by the `[acp.claude] models` configuration key; the session's current model SHALL be observed from the Claude wire frames that carry a model name. When an agent exposes no model option and no model surface of its own, and no current model can be observed, the model surface SHALL be absent (no dropdown), not an error. A session whose current model is observable but that offers no switchable options SHALL present the model chip as a read-only display of the current model rather than a "no models" placeholder.

This session-scoped list SHALL be authoritative for switching within that
session. Choosing the model of a session that does not exist yet is a separate
surface — the composer's catalog selector, fed by the operator's Settings
configuration (see `agent-workbench`) — and SHALL NOT be derived from another
session's list.

#### Scenario: ConfigOptions model list feeds the dropdown

- **WHEN** an ACP agent returns a `model` config option listing selectable models
- **THEN** the session exposes those model ids plus the current value
- **AND** the webui session form renders a model dropdown populated from that list

#### Scenario: Claude session exposes the alias table

- **WHEN** a Claude-backed session is created and the driver's handshake completes
- **THEN** the session exposes the built-in Claude model aliases (or the `[acp.claude] models` override) as selectable options, with the observed current model as the selection

#### Scenario: configuration overrides the alias table

- **WHEN** `[acp.claude] models` lists specific model ids
- **THEN** Claude sessions expose exactly that list instead of the built-in aliases

#### Scenario: observed current model renders read-only

- **WHEN** a session reports a current model but offers no switchable options
- **THEN** the model chip displays the current model as read-only status and no model error is raised

#### Scenario: Agent without model option shows no dropdown

- **WHEN** an ACP agent returns no `model` config option and no current model is observable
- **THEN** no model dropdown is shown and no model error is raised

#### Scenario: session list and creation-time catalog are independent

- **WHEN** the operator views a session whose agent exposes no model option, while
  providers with model lists are configured in Settings
- **THEN** the session offers no model dropdown and no model error, and the
  creation-time selector still offers the configured catalog for the next session

### Requirement: Model change via session/set_config_option

The system SHALL implement model switching on an ACP session by issuing the standard ACP `session/set_config_option` with `configId = "model"` and the chosen model id; the Claude-specific driver SHALL implement the same operator-facing semantic over its own control protocol (`set_model`). A rejected or unknown model value SHALL surface an explicit error and SHALL NOT change the session's current model silently. A successful Claude switch SHALL be reflected optimistically in the session's current model and SHALL be superseded by the next wire frame that carries a model name.

#### Scenario: Selecting a model switches the session

- **WHEN** the user picks a model from the session's list
- **THEN** the driver sends `session/set_config_option {configId:"model", value:<chosen>}` (or the Claude control-protocol equivalent)
- **AND** on success the session reports the new model

#### Scenario: Invalid model is rejected explicitly

- **WHEN** the agent rejects the model value (unknown id)
- **THEN** the caller receives an explicit error naming the model
- **AND** the session's current model is unchanged

#### Scenario: Claude switch applies from the next turn

- **WHEN** the operator switches a Claude session's model mid-session
- **THEN** the driver issues the control-protocol model switch, the session's current model updates, and subsequent prompts run under the chosen model

### Requirement: Model selection survives into the session lifecycle

A model chosen at session creation SHALL be applied to that session; a model chosen mid-session SHALL apply to subsequent turns. The model SHALL be part of the session's descriptive snapshot where the webui reads it. The spawn-time model cover env (`claude-env-cover`) SHALL establish the session's starting model on the Claude child; a runtime switch through `session/set_config_option` (or the Claude-equivalent model switch) SHALL take effect from the moment it succeeds and SHALL NOT be reverted by the spawn-time env on later turns.

#### Scenario: Create-with-model applies at spawn

- **WHEN** a session is created with a requested model
- **THEN** the driver applies the model via `session/set_config_option` once the session is established, before or with the first prompt
- **AND** the session starts under the model cover env derived at spawn

#### Scenario: Mid-session switch applies to later turns

- **WHEN** the user changes the model during an active session
- **THEN** subsequent prompts run under the new model
- **AND** the new model is not overwritten back to the spawn-time env value on later turns of the same child
