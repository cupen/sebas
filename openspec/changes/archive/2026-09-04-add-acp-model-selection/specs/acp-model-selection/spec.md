## Purpose

Lets users pick the model an ACP-backed session runs on. The agent exposes its model choices over standard ACP `configOptions` and accepts switches via `session/set_config_option`; sebas surfaces that list as a per-session model dropdown and forwards the user's choice to the child process. Works for any native ACP agent that advertises a model config option (opencode, gemini, …), with no agent-specific code.

## ADDED Requirements

### Requirement: Session model list is exposed

For each ACP session, the system SHALL expose the set of selectable models reported by the agent via its `configOptions` (the `model`-category option's select values), together with the session's current model. The source SHALL be the agent's own `session/new`/`session/load` response — never a hardcoded list. When the agent exposes no model option, the model surface SHALL be absent (no dropdown), not an error.

#### Scenario: ConfigOptions model list feeds the dropdown

- **WHEN** an ACP agent returns a `model` config option listing selectable models
- **THEN** the session exposes those model ids plus the current value
- **AND** the webui session form renders a model dropdown populated from that list

#### Scenario: Agent without model option shows no dropdown

- **WHEN** an ACP agent returns no `model` config option
- **THEN** no model dropdown is shown and no model error is raised

### Requirement: Model change via session/set_config_option

The system SHALL implement model switching on an ACP session by issuing the standard ACP `session/set_config_option` with `configId = "model"` and the chosen model id. A rejected or unknown model value SHALL surface an explicit error and SHALL NOT change the session's current model silently.

#### Scenario: Selecting a model switches the session

- **WHEN** the user picks a model from the session's list
- **THEN** the driver sends `session/set_config_option {configId:"model", value:<chosen>}`
- **AND** on success the session reports the new model

#### Scenario: Invalid model is rejected explicitly

- **WHEN** the agent rejects the model value (unknown id)
- **THEN** the caller receives an explicit error naming the model
- **AND** the session's current model is unchanged

### Requirement: Model selection survives into the session lifecycle

A model chosen at session creation SHALL be applied to that session; a model chosen mid-session SHALL apply to subsequent turns. The model SHALL be part of the session's descriptive snapshot where the webui reads it.

#### Scenario: Create-with-model applies at spawn

- **WHEN** a session is created with a requested model
- **THEN** the driver applies the model via `session/set_config_option` once the session is established, before or with the first prompt

#### Scenario: Mid-session switch applies to later turns

- **WHEN** the user changes the model during an active session
- **THEN** subsequent prompts run under the new model