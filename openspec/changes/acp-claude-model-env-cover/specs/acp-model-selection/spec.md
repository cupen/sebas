## MODIFIED Requirements

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
