## MODIFIED Requirements

### Requirement: Session model list is exposed

For each ACP session, the system SHALL expose the set of selectable models reported by the agent via its `configOptions` (the `model`-category option's select values), together with the session's current model. The source for generic ACP agents SHALL be the agent's own `session/new`/`session/load` response — never a hardcoded list. The Claude-specific driver — whose control protocol has no config-options channel — SHALL instead expose the agent's model slot mapping when one is configured: a `default` entry plus one entry per enabled slot (`fable`/`opus`/`sonnet`/`haiku`); without a mapping it SHALL expose the built-in fallback table `default`, `fable`, `opus`, `sonnet`, `haiku`. The session's current model SHALL be observed from the Claude wire frames that carry a model name. When an agent exposes no model option and no model surface of its own, and no current model can be observed, the model surface SHALL be absent (no dropdown), not an error. A session whose current model is observable but that offers no switchable options SHALL present the model chip as a read-only display of the current model rather than a "no models" placeholder.

This session-scoped list SHALL be authoritative for switching within that
session. Choosing the model of a session that does not exist yet is a separate
surface: for a claude agent the creation-time selector SHALL offer that agent's
slot list (`default` plus enabled slots, or the fallback table when no mapping
exists), preselecting the slot marked default — never a remembered previous
choice; for any other agent the creation-time selector remains the composer's
catalog selector, fed by the operator's Settings configuration (see
`agent-workbench`), and SHALL NOT be derived from another session's list.

#### Scenario: ConfigOptions model list feeds the dropdown

- **WHEN** an ACP agent returns a `model` config option listing selectable models
- **THEN** the session exposes those model ids plus the current value
- **AND** the webui session form renders a model dropdown populated from that list

#### Scenario: Claude session exposes the alias table

- **WHEN** a Claude-backed session is created for an agent whose mapping enables `fable`/`opus`/`sonnet` with `sonnet` default, and the driver's handshake completes
- **THEN** the session exposes `default`, `fable`, `opus`, `sonnet` as selectable options, with the observed current model as the selection

#### Scenario: configuration overrides the alias table

- **WHEN** the claude agent's slot mapping is configured (via the Settings form or `[acp.agents.<id>]` in config.toml)
- **THEN** Claude sessions expose exactly `default` plus the enabled slots instead of the built-in fallback table

#### Scenario: Claude agent without mapping exposes the fallback table

- **WHEN** a Claude-backed session is created for an agent with no slot mapping
- **THEN** the session exposes the built-in fallback table `default`/`fable`/`opus`/`sonnet`/`haiku`

#### Scenario: Claude creation-time selector preselects the default slot

- **WHEN** the operator selects a claude agent in the create-session dialog and that agent's mapping marks `sonnet` default
- **THEN** the dialog's model dropdown offers `default` plus the enabled slots with `sonnet` preselected
- **AND** the preselection ignores any previously remembered (provider, model) choice

#### Scenario: Non-claude creation-time selector is unchanged

- **WHEN** the operator selects a non-claude agent in the create-session dialog while providers with model lists are configured in Settings
- **THEN** the dialog's model dropdown still offers the configured catalog as before

#### Scenario: observed current model renders read-only

- **WHEN** a session reports a current model but offers no switchable options
- **THEN** the model chip displays the current model as read-only status and no model error is raised

#### Scenario: Agent without model option shows no dropdown

- **WHEN** an ACP agent returns no `model` config option and no current model is observable
- **THEN** no model dropdown is shown and no model error is raised

#### Scenario: session list and creation-time catalog are independent

- **WHEN** the operator views a session whose agent exposes no model option, while providers with model lists are configured in Settings
- **THEN** the session offers no model dropdown and no model error, and the creation-time selector still offers the configured catalog for the next non-claude agent (and the slot list for the next claude agent), never a list derived from the viewed session
