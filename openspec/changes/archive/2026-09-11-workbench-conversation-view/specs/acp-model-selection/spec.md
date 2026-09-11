## MODIFIED Requirements

### Requirement: Session model list is exposed

For each ACP session, the system SHALL expose the set of selectable models reported by the agent via its `configOptions` (the `model`-category option's select values), together with the session's current model. The source SHALL be the agent's own `session/new`/`session/load` response — never a hardcoded list. When the agent exposes no model option, the model surface SHALL be absent (no dropdown), not an error.

This session-scoped list SHALL be authoritative for switching within that
session. Choosing the model of a session that does not exist yet is a separate
surface — the composer's catalog selector, fed by the operator's Settings
configuration (see `agent-workbench`) — and SHALL NOT be derived from another
session's list.

#### Scenario: ConfigOptions model list feeds the dropdown

- **WHEN** an ACP agent returns a `model` config option listing selectable models
- **THEN** the session exposes those model ids plus the current value
- **AND** the webui session form renders a model dropdown populated from that list

#### Scenario: Agent without model option shows no dropdown

- **WHEN** an ACP agent returns no `model` config option
- **THEN** no model dropdown is shown and no model error is raised

#### Scenario: session list and creation-time catalog are independent

- **WHEN** the operator views a session whose agent exposes no model option, while
  providers with model lists are configured in Settings
- **THEN** the session offers no model dropdown and no model error, and the
  creation-time selector still offers the configured catalog for the next session
