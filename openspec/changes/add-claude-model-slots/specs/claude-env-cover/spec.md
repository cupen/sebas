## ADDED Requirements

### Requirement: Model env from agent slot mapping

At spawn of a Claude Code child whose agent definition carries a model slot mapping, the system SHALL set on the child's environment, overriding any value inherited from the OS environment: `ANTHROPIC_MODEL` equal to the model id of the slot marked default; `ANTHROPIC_DEFAULT_FABLE_MODEL`, `ANTHROPIC_DEFAULT_OPUS_MODEL`, `ANTHROPIC_DEFAULT_SONNET_MODEL`, and `ANTHROPIC_DEFAULT_HAIKU_MODEL` equal to the mapped model id of each enabled slot; and `CLAUDE_CODE_SUBAGENT_MODEL` equal to the model id of the weakest enabled slot (tier order fable > opus > sonnet > haiku). A slot that is not enabled SHALL NOT have its variable injected. The injection SHALL NOT depend on the provider mode — Direct, Router, and Off spawn identically, because the mapping values are ordinary model-catalog entries (a router alias is selectable the same as any provider model). When the claude agent definition carries no slot mapping, no model variable SHALL be forced and the child keeps whatever it inherits plus its own defaults.

#### Scenario: Full mapping injects the complete variable set

- **WHEN** a claude agent maps all four slots and marks `sonnet` default
- **THEN** `ANTHROPIC_MODEL` equals the sonnet slot's model id
- **AND** all four `ANTHROPIC_DEFAULT_*_MODEL` variables carry their slot's model id
- **AND** `CLAUDE_CODE_SUBAGENT_MODEL` equals the haiku slot's model id

#### Scenario: Disabled slot omits its variable

- **WHEN** a mapping enables `fable`/`opus`/`sonnet` with `haiku` disabled and `opus` marked default
- **THEN** no `ANTHROPIC_DEFAULT_HAIKU_MODEL` is injected
- **AND** `CLAUDE_CODE_SUBAGENT_MODEL` equals the sonnet slot's model id (weakest enabled tier)

#### Scenario: Mapping applies identically in Router mode

- **WHEN** provider mode is `Router` and the claude agent carries a slot mapping
- **THEN** the child receives the same model variables as a Direct-mode spawn of the same agent

#### Scenario: No mapping forces nothing

- **WHEN** a claude agent carries no slot mapping, in any provider mode
- **THEN** no model variable is injected and the child may use its own defaults

### Requirement: Override beats inherited values

On every Claude spawn whose agent carries a slot mapping, the injected model variables SHALL override — never merely fill a gap — any value already present in the OS environment, so a shell export or a Claude settings file cannot silently change the model the child runs on.

#### Scenario: Inherited value is replaced

- **WHEN** the OS environment carries `ANTHROPIC_MODEL=some-other-model` and the mapping's default slot holds model id `glm-5.3`
- **THEN** the child sees `ANTHROPIC_MODEL=glm-5.3`

## REMOVED Requirements

### Requirement: Model env cover set

**Reason**: the provider-derived strong→weak auto-ranking source is removed; the spawn-time model env contract is rewritten around the claude agent's slot mapping (see the added "Model env from agent slot mapping" requirement).
**Migration**: configure the claude agent's model slot mapping in Settings → Agents (gear customization) or under `[acp.agents.<id>]` in config.toml; provider model lists remain available as the catalog the mapping values are picked from.

### Requirement: Cover derivation from the effective provider's model list

**Reason**: the provider-derived strong→weak auto-ranking is removed; the sole source of model environment variables is the claude agent's slot mapping.
**Migration**: configure the claude agent's model slot mapping in Settings → Agents (gear customization) or under `[acp.agents.<id>]` in config.toml; provider model lists remain available as the catalog the mapping values are picked from.

### Requirement: Model flag parity with cover env

**Reason**: the provider-derived `--model` argv channel is retired together with the auto-ranking — spawn-time model pinning for claude happens solely through the mapping-derived `ANTHROPIC_MODEL`.
**Migration**: the default slot's model id takes over the role the `--model` flag used to play.

### Requirement: Transparency across provider modes

**Reason**: superseded by the modified "Model env cover set" requirement, which mandates identical injection in every provider mode; the Router-mode suppression special case no longer exists.
**Migration**: none — Router-mode agents previously never received cover variables; they now receive exactly what their mapping (if any) declares.
