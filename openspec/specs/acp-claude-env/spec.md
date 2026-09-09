# acp-claude-env Specification

## Purpose

Defines the model environment variables sebas injects into every Claude Code child it spawns, derived at spawn time from the effective provider's model list so the child runs the operator-chosen model family regardless of what the shell or the user's Claude settings would otherwise supply.

## Requirements

### Requirement: Model env cover set

At spawn of a Claude Code child in a mode that resolves a provider's model set, the system SHALL set each of the following environment variables on the child's environment, overriding any value inherited from the OS environment: `ANTHROPIC_MODEL`, `ANTHROPIC_DEFAULT_OPUS_MODEL`, `ANTHROPIC_DEFAULT_SONNET_MODEL`, `ANTHROPIC_DEFAULT_HAIKU_MODEL`, and `CLAUDE_CODE_SUBAGENT_MODEL`. The four `ANTHROPIC_*` values SHALL come from the provider's strong→weak model list per the existing `map_to_env` mapping; `CLAUDE_CODE_SUBAGENT_MODEL` SHALL equal the weakest model id (the haiku-tier value). When the provider's model list is empty, none of these variables SHALL be forced.

#### Scenario: Five cover vars from a preset provider

- **WHEN** provider mode resolves a provider whose model list is `["deepseek-v4-pro[1m]", "deepseek-v4-flash"]`
- **THEN** `ANTHROPIC_MODEL` and `ANTHROPIC_DEFAULT_OPUS_MODEL` equal the first id
- **AND** `ANTHROPIC_DEFAULT_SONNET_MODEL` equals the second id
- **AND** `ANTHROPIC_DEFAULT_HAIKU_MODEL` and `CLAUDE_CODE_SUBAGENT_MODEL` equal the weakest id

#### Scenario: Single-model provider flattens all tiers

- **WHEN** the provider's model list holds exactly one model id
- **THEN** all five cover variables carry that same id

#### Scenario: No model list forces nothing

- **WHEN** the resolved provider carries no model list
- **THEN** no cover variable is injected and the child may use its own defaults

### Requirement: Cover derivation from the effective provider's model list

The model cover values SHALL be derived at spawn time from the same provider resolution that selects the child's `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN`: the effective provider's strong→weak model list (preset-materialized or user-written) feeds the tier mapping through the existing `map_to_env` logic. The derivation SHALL NOT change the provider data structure. The mapping itself remains open for a future capability-tier annotation (e.g. T0/T1/T2 labels overriding the strong→weak order); until such annotation exists the strong→weak list order is the tier mapping.

#### Scenario: Direct mode derives from provider models

- **WHEN** Direct mode resolves preset provider `deepseek` with models `["deepseek-v4-pro[1m]", "deepseek-v4-flash"]`
- **THEN** the child receives all five cover variables mapped from that list

#### Scenario: Off mode with default derives implicit-Direct coverage

- **WHEN** mode is `off`, `default_selection` names a provider with a model list, and spawn runs
- **THEN** the child receives the five cover variables mapped from that provider's model list

### Requirement: Model flag parity with cover env

When the same resolution path would pass a model id as the `--model` CLI argument (Direct/Off-with-default per provider-management precedence), the `ANTHROPIC_MODEL` cover value SHALL be the provider's strongest model id — the same id the router's `default_model` accessor returns — so the env and flag agree when both are emitted.

#### Scenario: Cover env and --model flag agree

- **WHEN** Direct resolution yields `--model deepseek-v4-pro[1m]`
- **THEN** `ANTHROPIC_MODEL` in the child env is also `deepseek-v4-pro[1m]`

### Requirement: Override beats inherited values

On every Claude spawn in a covering mode, the five cover variables SHALL override — never merely fill a gap — any value already present in the OS environment, so a shell export or a Claude settings file cannot silently change the model the child runs on.

#### Scenario: Inherited value is replaced

- **WHEN** the OS environment carries `ANTHROPIC_MODEL=some-other-model` and the effective provider's strongest model is `deepseek-v4-pro[1m]`
- **THEN** the child sees `ANTHROPIC_MODEL=deepseek-v4-pro[1m]`

### Requirement: Transparency across provider modes

The cover derivation SHALL apply in Direct and Off-with-default modes, which explicitly select a provider. Router mode SHALL NOT inject the cover variables: the router's role is transparent proxying, and pinning a model there contradicts the operator choosing routes by model. An Off-mode spawn with no default selection SHALL leave the child's own model discovery untouched.

#### Scenario: Router-mode spawn carries no cover

- **WHEN** provider mode is `Router` and a Claude child is spawned
- **THEN** no cover variable is injected and the child may use its own model defaults

#### Scenario: Bare Off leaves discovery alone

- **WHEN** provider mode is `off`, no default selection exists, and a Claude child is spawned
- **THEN** no cover variable is injected
