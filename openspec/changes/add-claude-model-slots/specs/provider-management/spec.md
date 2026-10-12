## MODIFIED Requirements

### Requirement: Off mode resolution

With mode `Off` and no default selection, the spawn SHALL pass no provider endpoint env vars (`ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` / OpenAI equivalents) — the ACP child uses its own discovered endpoint configuration. With a default selection present the spawn SHALL behave exactly as implicit `Direct` mode for the selected provider, for the endpoint env vars. Model env SHALL follow the `claude-env-cover` contract — derived from the claude agent's slot mapping, independent of provider mode — and the spawn SHALL NOT construct any `--model` argument in any mode.

#### Scenario: bare off mode

- **WHEN** the state file has mode `off` and no default selection
- **THEN** the spawned child receives no provider endpoint env vars and no `--model` arg; model variables follow the claude-env-cover contract (agent slot mapping) untouched by this mode

#### Scenario: implicit direct

- **WHEN** mode is `off` but `default_selection` names provider `alpha`
- **THEN** the spawn resolves `alpha` as in Direct mode for the endpoint env vars, passes no `--model` arg, and model variables still follow the claude agent's slot mapping

## REMOVED Requirements

### Requirement: Model flag precedence

**Reason**: the provider-derived `--model` argv channel is retired — claude spawn-time model pinning now comes solely from the agent's model slot mapping through env (see the `claude-env-cover` delta); provider metadata (`default_selection.model`, `default_model`) no longer pins a claude child's model.
**Migration**: configure the claude agent's model slot mapping; the default slot's model id takes over the pinning role via `ANTHROPIC_MODEL`. Provider `default_model` remains provider metadata for the catalog and probing surfaces.
