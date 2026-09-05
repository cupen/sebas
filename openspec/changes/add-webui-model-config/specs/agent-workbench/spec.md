## ADDED Requirements

### Requirement: Model selector sources from the backend catalog

The composer's model selector SHALL source the native backend's model options
from the backend-level model catalog (derived from provider configuration) —
available before any session exists — rather than from whichever session
happens to expose a model list. ACP sessions SHALL continue to expose their
agent-declared model options for their own selector. The selector SHALL update
when the catalog changes without requiring a session to be created first, and
SHALL state unavailability honestly when no catalog exists.

#### Scenario: selector populated before any session

- **WHEN** a default provider with a models catalog is configured and the
  operator opens a fresh workbench with no sessions
- **THEN** the model selector offers the catalog's models

#### Scenario: ACP keeps agent-declared options

- **WHEN** an ACP session exposes its agent's model options
- **THEN** the selector for that session offers exactly those options

#### Scenario: catalog change reaches the selector

- **WHEN** the operator edits the default provider's models catalog in
  settings while the workbench is open
- **THEN** the selector reflects the new catalog without creating a session or
  reloading the page

#### Scenario: no catalog states unavailability

- **WHEN** no provider catalog is configured
- **THEN** the selector is absent or disabled with that cause stated, not
  silently empty
