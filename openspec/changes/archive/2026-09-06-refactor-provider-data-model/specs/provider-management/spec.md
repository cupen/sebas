## ADDED Requirements

### Requirement: Preset data follows the code table

The built-in preset table SHALL remain hardcoded in the application code.
A provider that selects a preset (by name or explicit `preset` field)
SHALL resolve its base URLs and models catalog from that table at
load/resolve time — never from a persisted copy — so an application update
that changes a preset's data automatically applies to every provider
derived from it without any stored-data change. The only user-owned fields
on a preset-derived provider are its API key (or `api_key_env`), its
default model, and its Direct-spawn `protocol` preference. An explicit
base URL or models override on a preset-derived provider SHALL be a
configuration error, not a silent overwrite of preset data.

#### Scenario: derived provider resolves from code

- **WHEN** a provider entry names preset `deepseek` with no URL or models
  fields of its own
- **THEN** it resolves with the `deepseek` preset's base URLs and models
  from the built-in table

#### Scenario: code preset update propagates

- **WHEN** the application updates preset `kimi`'s URLs and the user
  restarts (or hot-reloads) without touching stored provider data
- **THEN** the preset-derived provider `kimi` serves the new URLs

#### Scenario: URL override rejected

- **WHEN** a preset-derived provider entry sets any explicit base URL slot
- **THEN** configuration parsing fails with an error explaining that
  preset-derived providers follow the code table

#### Scenario: api key stays user-owned

- **WHEN** the user stores an API key on a preset-derived provider and the
  application later ships a changed preset
- **THEN** the stored key is untouched by the preset update

## MODIFIED Requirements

### Requirement: Provider CRUD forms

The create/edit forms SHALL capture: preset form — name, preset selection,
API key (secret input), default model, protocol; custom form — the same
plus the three base URL slots (`base_url_anthropic`,
`base_url_openai_chat`, `base_url_openai_responses`) and `api_key_env`.
The preset form SHALL NOT offer base URL or models-catalog inputs:
preset-derived providers resolve those from the code table. A form submit
with an id matching an existing provider updates it; with an unknown id
inserts it; without an id a new provider is created. Deleting a provider
SHALL record a tombstone and clear the default selection if it pointed at
the deleted provider.

#### Scenario: preset form has no URL inputs

- **WHEN** the user opens the create-from-preset form for preset `glm`
- **THEN** the form offers name, preset, API key, default model, and
  protocol inputs only, with the preset's base URLs displayed as read-only
  code-owned values

#### Scenario: preset defaults normalizer

- **WHEN** the user creates a provider from preset `deepseek` submitting
  only a name and an API key
- **THEN** the stored provider carries no base URL or models fields — it
  resolves the preset's data from the code table — while a default model
  the user typed is preserved verbatim

#### Scenario: custom form captures three slots

- **WHEN** the user opens the custom-provider form
- **THEN** it offers all three base URL slots plus `api_key_env`, and
  validation rejects a submit that leaves all three slots empty

#### Scenario: delete clears default

- **WHEN** the user deletes provider `alpha` which is the current DIRECT
  default
- **THEN** the overlay records a tombstone for `alpha` and the default
  selection is cleared

### Requirement: Model probing

The probe-models button SHALL appear only for providers with at least one
OpenAI-family base URL slot configured. Probing issues a single `GET`
(5 s timeout) to one URL — `{base_url_openai_chat}/models` when set, else
`{base_url_openai_responses}/models`, else
`{base_url_anthropic}/v1/models` — authenticating with the stored API key
when present (plain-text key preferred, else the `api_key_env` value), and
parses the `data[].id` array. On success the full returned model list is
displayed on a separate result card whose `使用 <model>` buttons write that
model as the provider's default model. For custom providers the list is
also written back to the provider's models catalog; for preset-derived
providers nothing is persisted (the catalog follows the code table).
Probing failure renders a red error card with the reason.

#### Scenario: single-URL probe choice

- **WHEN** a provider has both `base_url_openai_chat` and
  `base_url_anthropic` set and the user clicks probe
- **THEN** only the OpenAI chat `/models` endpoint is queried; there is no
  fallback attempt against the Anthropic URL on failure

#### Scenario: catalog writeback and default selection

- **WHEN** the probe against a custom provider returns models `m1` and
  `m2` and the user clicks `使用 m2`
- **THEN** the provider's `models` catalog holds `["m1","m2"]` and its
  `default_model` is set to `m2`

#### Scenario: preset-derived probe is report-only

- **WHEN** the probe succeeds against a preset-derived provider
- **THEN** the result card lists the models and can set the default model,
  but the provider's stored data does not gain a models catalog

#### Scenario: probe without openai URL

- **WHEN** a provider defines only `base_url_anthropic`
- **THEN** no probe button is rendered on its panel

### Requirement: Direct mode env translation

In `Direct` mode the spawn SHALL resolve the selected provider from the
state overlay, falling back to the router config's provider table when the
name is absent from the overlay; a name found in neither place falls back to
Off-mode behavior (no env) rather than aborting. The resolved provider is
translated to env vars: Anthropic-protocol providers get
`ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN`; OpenAI-protocol providers
get `OPENAI_BASE_URL` + `OPENAI_API_KEY`, sourced from the
chat-completions slot (a Responses-only provider is not spawnable for
Direct mode and yields a resolution error). Protocol resolution prefers an
explicit `protocol` field (a missing required URL then aborts); with
`auto`, the Anthropic slot is preferred and the chat-completions slot used
only when the Anthropic URL is absent. Auth resolution prefers a stored
plain-text API key (used with a warning) and otherwise requires the
`api_key_env` variable to resolve — an unresolvable key aborts.

#### Scenario: anthropic env translation

- **WHEN** Direct mode resolves provider `alpha` with an Anthropic base URL
  and API key
- **THEN** the child env contains `ANTHROPIC_BASE_URL` and
  `ANTHROPIC_AUTH_TOKEN` for that provider

#### Scenario: openai env uses chat slot

- **WHEN** Direct mode resolves an OpenAI-protocol provider that sets both
  OpenAI slots
- **THEN** `OPENAI_BASE_URL` carries the chat-completions slot URL

#### Scenario: unknown provider falls back

- **WHEN** Direct mode names provider `ghost` that exists neither in the
  overlay nor the router config
- **THEN** the child spawns with no provider env (Off-like behavior) and no
  error is raised

#### Scenario: missing api key env aborts

- **WHEN** the provider's auth uses `api_key_env = "MISSING_VAR"` and that
  variable is unset in the spawn environment
- **THEN** the spawn resolves to an error carrying the reason
