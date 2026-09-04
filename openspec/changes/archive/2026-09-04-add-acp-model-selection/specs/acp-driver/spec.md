## ADDED Requirements

### Requirement: ACP config options surface on session establishment

The generic ACP driver SHALL parse the `configOptions` from a `session/new` or `session/load` response and expose the model option (id "model", its select values, and the current value) to the caller as part of the spawn outcome. The driver SHALL NOT hardcode or filter model lists; the agent's response is the source of truth.

#### Scenario: Spawn outcome carries the model option

- **WHEN** an ACP session is established and the agent returns a `model` config option
- **THEN** the spawn outcome carries the model select values and current value
- **AND** a session without a model option reports no model surface

### Requirement: ACP model switch command

The driver SHALL accept a session command to set the model and translate it to the standard ACP `session/set_config_option` (`configId = "model"`). A rejected value SHALL surface an explicit error; an agent without the method or option SHALL report the same explicit error.

#### Scenario: SetModel issues set_config_option

- **WHEN** a `SetModel { model_id }` command targets an ACP session
- **THEN** the driver sends `session/set_config_option` with the model value
- **AND** reports success or an explicit model-rejected error

#### Scenario: Unsupported agent reports explicit error

- **WHEN** the agent lacks `set_config_option`/the model option
- **THEN** the driver returns an explicit "model not supported" error instead of failing silently