## ADDED Requirements

### Requirement: Model selector offers the backend catalog before any session

The composer's model selector SHALL offer the backend catalog's models — the
models of the default provider (as set in the agent defaults) — before any
session exists, so the operator can pick a model for the first turn of a new
session. When sessions exist, the selector SHALL keep its existing behavior
(offering the `available_models` of the relevant session's execution body).
When neither a backend catalog nor a session model list is available, the
selector SHALL state its unavailability honestly rather than offering an empty
or fabricated list. Changes to the catalog or the default SHALL be reflected
without requiring a session to be created first.

#### Scenario: selector populated before any session

- **WHEN** a default provider with a models catalog is configured and the
  operator opens a fresh workbench with no sessions
- **THEN** the model selector offers the catalog's models

#### Scenario: existing sessions keep session-sourced options

- **WHEN** a session exposes `available_models` and the operator opens its
  model dropdown
- **THEN** the selector offers that session's options, matching the current
  behavior

#### Scenario: no catalog and no session models is stated honestly

- **WHEN** no default provider catalog exists and no session exposes models
- **THEN** the selector presents an explicit unavailability indication rather
  than an empty list
