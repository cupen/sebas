## ADDED Requirements

### Requirement: Execution-body availability is stated, not discovered

The composer's execution-body selector SHALL reflect, for each execution body,
whether it can serve new sessions in the current process configuration. An
execution body that cannot serve new sessions — for example the native kernel
running without provider credentials — SHALL be presented as unavailable with
its cause stated, and SHALL NOT be selectable such that the operator only
discovers the failure on submission. Availability SHALL be derived from the
session backend's own report of both execution bodies, not from the ACP side
alone.

#### Scenario: native kernel without credentials shown as unavailable

- **WHEN** the native kernel has no provider credentials and the composer is
  rendered
- **THEN** the `native` option is shown as unavailable with the cause stated,
  and submitting a native spawn is prevented at the composer rather than
  failing at the core

#### Scenario: both bodies available

- **WHEN** both the ACP bridge and the native kernel can serve new sessions
- **THEN** the selector offers both without degradation notices

#### Scenario: availability recovers without reload

- **WHEN** the cause making an execution body unavailable is resolved while the
  page stays open
- **THEN** the selector offers that body again without the operator reloading

### Requirement: Model selection covers the native kernel

The composer SHALL offer model selection for native-kernel sessions drawn from
the models the native execution body exposes, and a model chosen for a native
session SHALL apply to that session's subsequent turns. A native session with
no model selected SHALL use the kernel's default. Sessions on other execution
bodies keep their existing model-selection behavior unchanged.

#### Scenario: native session model dropdown is populated

- **WHEN** the operator opens the model dropdown for a native-kernel session
- **THEN** it lists the models the native execution body exposes rather than
  being empty

#### Scenario: chosen model applies to subsequent turns

- **WHEN** the operator selects a model on a native session and sends a message
- **THEN** the turn runs with the selected model, and the session's current
  model is visible afterwards

#### Scenario: unselected falls back to default

- **WHEN** a native session is created with no model chosen
- **THEN** its turns run with the native kernel's default model and the
  composer shows no false "selected" state
