# provider-management Specification

## Purpose
Covers the provider lifecycle as surfaced to the user: the `/provider` card
interaction model (mode switching, CRUD forms, model probing, secret masking)
and the spawn-time translation of the three provider modes into the env/args
handed to the ACP child process.

## Requirements

### Requirement: /provider main card layout

The `/provider` command SHALL render a single management card with four
sections, top to bottom: (1) three mode buttons `Off` / `Direct` / `Gateway`
with the current mode rendered `primary`; (2) a DIRECT-mode default provider
dropdown (options: all provider names in alphabetical order plus
`（未设置）`); (3) the provider list — one collapsed `collapsible_panel` per
provider whose header summarizes name, DIRECT-default mark, and default
model, and whose body shows markdown field rows (preset, base URLs, API key
configured-or-not, default model) plus four buttons (probe models, edit,
delete, set as DIRECT default); (4) a create sub-section with
`＋ 新增（预设）` and `＋ 新增（自定义）` buttons.

#### Scenario: card sections

- **WHEN** the user sends `/provider`
- **THEN** the bot sends one card containing the mode buttons, the default
  dropdown, the collapsed provider panels, and the two create buttons

#### Scenario: default dropdown options

- **WHEN** providers `zeta` and `alpha` exist and no default is selected
- **THEN** the dropdown offers `（未设置）`, `alpha`, `zeta` in that order

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

### Requirement: Provider CRUD forms

The create/edit forms SHALL capture: preset form — name, preset selection,
API key (secret input), models catalog, default model, protocol; custom form
— the same plus `base_url_anthropic`, `base_url_openai`, and
`api_key_env`. Applying a preset SHALL fill missing base URLs from the preset
definition and inject the preset's default `api_key_env`, without
overwriting a user-entered default model. A form submit with an id matching
an existing provider updates it; with an unknown id inserts it; without an
id a new provider is created. Deleting a provider SHALL record a tombstone
and clear the default selection if it pointed at the deleted provider.

#### Scenario: preset defaults normalizer

- **WHEN** the user creates a provider from preset `deepseek` and leaves
  both base URLs empty
- **THEN** the stored provider carries the preset's base URLs and
  `api_key_env`, while a default model the user typed is preserved verbatim

#### Scenario: delete clears default

- **WHEN** the user deletes provider `alpha` which is the current DIRECT
  default
- **THEN** the overlay records a tombstone for `alpha` and the default
  selection is cleared

### Requirement: Secret masking

Provider API keys SHALL never be echoed: the management card shows only
`已配置` / `未配置`; CRUD listings mask the key as `••••••`; edit forms never
pre-fill a secret field; submitting an empty secret field preserves the
stored key rather than clearing it.

#### Scenario: masked display

- **WHEN** the provider panel for `alpha` (which has an API key) renders
- **THEN** the body shows `API Key: 已配置` and no key material

#### Scenario: empty submit preserves key

- **WHEN** the user edits provider `alpha` and submits the form with the API
  key field left empty
- **THEN** the stored API key is unchanged

### Requirement: Model probing

The probe-models button SHALL appear only for providers with an OpenAI base
URL configured. Probing issues a single `GET` (5 s timeout) to one URL —
`{base_url_openai}/models` when set, else `{base_url_anthropic}/v1/models` —
authenticating with the stored API key when present (plain-text key
preferred, else the `api_key_env` value), and parses the `data[].id` array.
On success the full returned model list is written back to the provider's
models catalog and displayed on a separate result card whose
`使用 <model>` buttons write that model as the provider's default model;
probing failure renders a red error card with the reason.

#### Scenario: single-URL probe choice

- **WHEN** a provider has both `base_url_openai` and `base_url_anthropic`
  set and the user clicks probe
- **THEN** only the OpenAI `/models` endpoint is queried; there is no
  fallback attempt against the Anthropic URL on failure

#### Scenario: catalog writeback and default selection

- **WHEN** the probe returns models `m1` and `m2` and the user clicks
  `使用 m2`
- **THEN** the provider's `models` catalog holds `["m1","m2"]` and its
  `default_model` is set to `m2`

#### Scenario: probe without openai URL

- **WHEN** a provider defines only `base_url_anthropic`
- **THEN** no probe button is rendered on its panel

### Requirement: Off mode resolution

With mode `Off` and no default selection, the spawn SHALL pass no provider
env vars or model args — the ACP child uses its own discovered
configuration. With mode `Off` and a default selection present, the spawn
SHALL behave exactly as implicit `Direct` mode for the selected provider,
including the `--model` flag.

#### Scenario: bare off mode

- **WHEN** the state file has mode `off` and no default selection
- **THEN** the spawned child receives neither provider env vars nor a
  `--model` arg

#### Scenario: implicit direct

- **WHEN** mode is `off` but `default_selection` names provider `alpha`
- **THEN** the spawn resolves `alpha` as in Direct mode and applies the same
  `--model` precedence

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

### Requirement: Provider error abort

When provider resolution fails (error rather than fallback), the spawn SHALL
set only the `SEBAS_PROVIDER_ERROR` env var carrying the reason; the child
spawn wrapper SHALL print the reason and exit with code 1 before launching
the ACP binary.

#### Scenario: error aborts spawn

- **WHEN** provider resolution yields error reason `missing api key env`
- **THEN** the child process env contains `SEBAS_PROVIDER_ERROR` with that
  reason, the wrapper prints it to stderr, and the process exits 1 without
  running the agent

### Requirement: Provider card reflects store availability

The `/provider` card SHALL render normally while the state store is reachable. When the store is unavailable or corrupt, the card SHALL present an explicit unavailable state with the cause, disable mutation entry points, and leave all user data untouched.

#### Scenario: Store unavailable shows cause

- **WHEN** the state store is unreachable while a `/provider` card flow is active
- **THEN** the card renders an explicit unavailable state naming the cause, with mutations disabled

#### Scenario: No silent data loss from the card path

- **WHEN** the state store reports corruption
- **THEN** no card-driven operation deletes or resets provider data; recovery goes through the documented manual paths
