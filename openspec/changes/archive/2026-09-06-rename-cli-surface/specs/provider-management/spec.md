## RENAMED Requirements

- FROM: `### Requirement: Gateway mode env translation`
- TO: `### Requirement: Router mode env translation`

## MODIFIED Requirements

### Requirement: Mode switching

Clicking a mode button SHALL write the mode to the state file and refresh the
management card in place. Switching to `Direct` while no default provider is
selected SHALL auto-fill the alphabetically-first provider as the default
(without setting a default model). The persisted mode value SHALL be
`router`; a state file carrying the pre-rename value `gateway` SHALL still
parse as `Router` mode.

#### Scenario: direct auto-fill

- **WHEN** the user clicks `Direct` with providers `alpha` and `zeta`
  configured and no default selection
- **THEN** the state file records mode `direct` with default provider
  `alpha`, and the refreshed card marks `alpha` as the DIRECT default

#### Scenario: gateway mode write

- **WHEN** the user clicks `Router`
- **THEN** the state file records mode `router` and the card refreshes with
  `Router` rendered as the active mode

#### Scenario: legacy state value still loads

- **WHEN** the state file contains `"kind": "gateway"` from a pre-rename
  version
- **THEN** the state loads with `Router` mode active and no error

### Requirement: Direct mode env translation

In `Direct` mode the spawn SHALL resolve the selected provider from the
state overlay, falling back to the router config's provider table when the
name is absent from the overlay; a name found in neither place falls back to
Off-mode behavior (no env) rather than aborting. The resolved provider is
translated to env vars: Anthropic-protocol providers get
`ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN`; OpenAI-protocol providers get
`OPENAI_BASE_URL` + `OPENAI_API_KEY`. Protocol resolution prefers an
explicit `protocol` field (a missing required URL then aborts); with
`auto`, the Anthropic base URL is preferred and OpenAI used only when the
Anthropic URL is absent. Auth resolution prefers a stored plain-text API key
(used with a warning) and otherwise requires the `api_key_env` variable to
resolve — an unresolvable key aborts.

#### Scenario: anthropic env translation

- **WHEN** Direct mode resolves provider `alpha` with an Anthropic base URL
  and API key
- **THEN** the child env contains `ANTHROPIC_BASE_URL` and
  `ANTHROPIC_AUTH_TOKEN` for that provider

#### Scenario: unknown provider falls back

- **WHEN** Direct mode names provider `ghost` that exists neither in the
  overlay nor the router config
- **THEN** the child spawns with no provider env (Off-like behavior) and no
  error is raised

#### Scenario: missing api key env aborts

- **WHEN** the provider's auth uses `api_key_env = "MISSING_VAR"` and that
  variable is unset in the spawn environment
- **THEN** the spawn resolves to an error carrying the reason

### Requirement: Model flag precedence

When constructing the `--model` arg, the spawn SHALL apply precedence:
`default_selection.model` (only when the default selection's provider is the
resolved provider) over the provider's `default_model`, over no flag. In
Router mode the `--model` flag SHALL never be added.

#### Scenario: default selection wins

- **WHEN** `default_selection` is provider `alpha` model `m1` and `alpha`'s
  `default_model` is `m2`
- **THEN** the child receives `--model m1`

#### Scenario: overlay default model fallback

- **WHEN** `default_selection` names provider `alpha` with no model and
  `alpha`'s `default_model` is `m2`
- **THEN** the child receives `--model m2`

#### Scenario: gateway never pins model

- **WHEN** mode is `router` and both default selection model and provider
  default model exist
- **THEN** no `--model` arg is passed

### Requirement: Router mode env translation

In `Router` mode the spawn SHALL translate the router's `listen` address
and first `auth_token` into `ANTHROPIC_BASE_URL=http://{listen}` +
`ANTHROPIC_AUTH_TOKEN={token}` — the router always presents the Anthropic
protocol face to the agent. An empty `listen` is a resolution error; an
empty `auth_token` proceeds with a warning.

#### Scenario: gateway env construction

- **WHEN** mode is `router` with `listen = "127.0.0.1:8787"` and
  `auth_token = ["sk-x"]`
- **THEN** the child env sets `ANTHROPIC_BASE_URL=http://127.0.0.1:8787` and
  `ANTHROPIC_AUTH_TOKEN=sk-x`

#### Scenario: missing listen aborts

- **WHEN** mode is `router` and the router config has an empty `listen`
- **THEN** the spawn resolves to an error and the child is not started with
  partial router env
