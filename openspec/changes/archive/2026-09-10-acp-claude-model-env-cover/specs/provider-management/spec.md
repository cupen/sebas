## MODIFIED Requirements

### Requirement: Off mode resolution

With mode `Off` and no default selection, the spawn SHALL pass no provider endpoint env vars (`ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` / OpenAI equivalents) — the ACP child uses its own discovered endpoint configuration. Model cover env SHALL follow the `claude-env-cover` contract: with a default selection present the spawn SHALL behave exactly as implicit `Direct` mode for the selected provider, including the `--model` flag and the model cover env; without a default selection no cover variable is forced.

#### Scenario: bare off mode

- **WHEN** the state file has mode `off` and no default selection
- **THEN** the spawned child receives no provider endpoint env vars, no `--model` arg, and no model cover variable

#### Scenario: implicit direct

- **WHEN** mode is `off` but `default_selection` names provider `alpha`
- **THEN** the spawn resolves `alpha` as in Direct mode, applies the same `--model` precedence, and derives the model cover env from that provider's model list

### Requirement: Model flag precedence

When constructing the `--model` arg, the spawn SHALL apply precedence: `default_selection.model` (only when the default selection's provider is the resolved provider) over the provider's `default_model`, over no flag. In Router mode the `--model` flag SHALL never be added. When a `--model` id is emitted and the provider also yields cover env, the `ANTHROPIC_MODEL` cover value SHALL be the provider's strongest model id, matching the id `--model` carries when the user's default selection is unset; a user-typed `default_selection.model` stays the `--model` source while the cover tracks the provider list (the two may differ by intent — the user overriding the spawn flag does not rewrite the provider's tier list).

#### Scenario: default selection wins

- **WHEN** `default_selection` is provider `alpha` model `m1` and `alpha`'s `default_model` is `m2`
- **THEN** the child receives `--model m1`
- **AND** the cover env still maps from provider `alpha`'s model list, not from `m1`

#### Scenario: overlay default model fallback

- **WHEN** `default_selection` names provider `alpha` with no model and `alpha`'s `default_model` is `m2`
- **THEN** the child receives `--model m2`

#### Scenario: router never pins model

- **WHEN** mode is `router` and both default selection model and provider default model exist
- **THEN** no `--model` arg is passed
- **AND** no model cover variable is injected

#### Scenario: no resolvable model

- **WHEN** no model can be resolved by the precedence chain and the provider yields no model list
- **THEN** no `--model` arg is passed and no cover variable is forced
