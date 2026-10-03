## MODIFIED Requirements

### Requirement: Model change via session/set_config_option

The system SHALL implement model switching on an ACP session by issuing the standard ACP `session/set_config_option` with `configId = "model"` and the chosen model id; the Claude-specific driver SHALL implement the same operator-facing semantic over its own control protocol (`set_model`). A rejected or unknown model value SHALL surface an explicit error and SHALL NOT change the session's current model silently. A successful Claude switch SHALL be reflected optimistically in the session's current model and SHALL be superseded by the next wire frame that carries a model name. The workbench composer's model selector SHALL be wired to this contract: picking a model from the composer MUST actually dispatch the switch (not merely update local UI state), and a rejection MUST surface the typed error to the operator with the selection rolled back to the session's effective model.

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

#### Scenario: Composer picking a reject-listed model surfaces the typed rejection

- **WHEN** an ACP session's agent advertises model options where a listed model is reject-listed (fakeacp's `bad-model` with `--reject-model`), and the operator selects it in the composer
- **THEN** the switch request is dispatched and the operator sees an explicit rejection naming the model (no silent success, no normal turn under the rejected model)
- **AND** the composer's model selection falls back to the session's effective model
