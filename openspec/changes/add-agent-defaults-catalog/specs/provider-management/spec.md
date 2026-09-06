## ADDED Requirements

### Requirement: Set default provider and model from the page

The provider management page SHALL let the operator mark a provider as the
default for new sessions and pick that provider's default model, and SHALL
show which provider and model are currently the default. Setting the default
SHALL persist across restarts (router-side, alongside the provider store) and
SHALL NOT alter any existing session's model. Clearing the default SHALL
return new sessions to the execution body's built-in default.

#### Scenario: set default from the page

- **WHEN** the operator marks provider `glm` with model `m2` as the default
- **THEN** the page shows `glm` / `m2` as the current default, and the value
  survives a router restart

#### Scenario: existing sessions are untouched

- **WHEN** the default changes while a session runs with its own selected
  model
- **THEN** that session keeps its selected model for subsequent turns

#### Scenario: clear the default

- **WHEN** the operator clears the default
- **THEN** new sessions use their execution body's built-in default and the
  page shows no default
