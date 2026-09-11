# provider-management Specification

## Purpose
Covers the provider lifecycle as surfaced to the user: the `/provider` card
interaction model (mode switching, CRUD forms, model probing, secret masking)
and the spawn-time translation of the three provider modes into the env/args
handed to the ACP child process.

## Requirements

### Requirement: /provider main card layout

The `/provider` command SHALL render a single management card with four
sections, top to bottom: (1) three mode buttons `Off` / `Direct` / `Router`
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
`router`; a state file carrying the pre-rename value `gateway` SHALL fail
to parse — nothing was released under the old value, so no alias is kept.

#### Scenario: direct auto-fill

- **WHEN** the user clicks `Direct` with providers `alpha` and `zeta`
  configured and no default selection
- **THEN** the state file records mode `direct` with default provider
  `alpha`, and the refreshed card marks `alpha` as the DIRECT default

#### Scenario: router mode write

- **WHEN** the user clicks `Router`
- **THEN** the state file records mode `router` and the card refreshes with
  `Router` rendered as the active mode

#### Scenario: pre-rename state value rejected

- **WHEN** the state file contains `"kind": "gateway"`
- **THEN** parsing fails with an error naming the state file instead of
  silently guessing the mode

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

The probe-models button SHALL appear only for providers with at least one
OpenAI-family base URL slot configured. Probing is performed by core on behalf of the
requesting surface: it issues a single `GET` (5 s timeout) to one URL —
`{base_url_openai_chat}/models` when set, else `{base_url_openai_responses}/models`,
else `{base_url_anthropic}/v1/models` — authenticating with the provider's resolved
key (plain-text key preferred, else the `api_key_env` value), and parses the
`data[].id` array. It SHALL be available for preset-derived providers as well as
custom ones, using the preset's code-table base URLs. On success the full returned
model list is displayed on a separate result card whose `使用 <model>` buttons write
that model as the provider's default model. For custom providers the list is also
written back to the provider's models catalog; for preset-derived providers nothing is
persisted, because the fetch itself modifies no provider field. The upstream response
carries model ids only: context window and similar parameters SHALL be resolved from
the local static table by id, and an unknown id SHALL fall back to the documented
default and be labelled unknown rather than guessed from its name. Neither the result
card nor any error message SHALL contain key material. Probing failure renders a red
error card with the sanitized reason.

#### Scenario: single-URL probe choice

- **WHEN** a provider has both `base_url_openai_chat` and
  `base_url_anthropic` set and the user clicks probe
- **THEN** only the OpenAI chat `/models` endpoint is queried; there is no
  fallback attempt against the Anthropic URL on failure

#### Scenario: catalog writeback and default selection

- **WHEN** the probe against a custom provider returns models `m1` and
  `m2` and the user clicks `使用 m2`
- **THEN** the provider's model list contains `m1` and `m2` and its
  `default_model` is set to `m2`

#### Scenario: preset-derived probe is report-only

- **WHEN** the probe succeeds against a preset-derived provider
- **THEN** the result card lists the models and can set the default model,
  but the fetch writes no provider field and the preset's code-table data is unchanged

#### Scenario: probe without openai URL

- **WHEN** a provider defines only `base_url_anthropic`
- **THEN** no probe button is rendered on its panel

#### Scenario: upstream parameters are not invented

- **WHEN** the probe returns a model id absent from the local static table
- **THEN** the result entry carries the id with its parameters marked unknown and the documented default applied, rather than a value derived from the model's name

#### Scenario: key material never reaches the result

- **WHEN** the probe succeeds or fails
- **THEN** neither the result card nor the error text contains the provider's key or the `api_key_env` value

### Requirement: Off mode resolution

With mode `Off` and no default selection, the spawn SHALL pass no provider endpoint env vars (`ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` / OpenAI equivalents) — the ACP child uses its own discovered endpoint configuration. Model cover env SHALL follow the `claude-env-cover` contract: with a default selection present the spawn SHALL behave exactly as implicit `Direct` mode for the selected provider, including the `--model` flag and the model cover env; without a default selection no cover variable is forced.

#### Scenario: bare off mode

- **WHEN** the state file has mode `off` and no default selection
- **THEN** the spawned child receives no provider endpoint env vars, no `--model` arg, and no model cover variable

#### Scenario: implicit direct

- **WHEN** mode is `off` but `default_selection` names provider `alpha`
- **THEN** the spawn resolves `alpha` as in Direct mode, applies the same `--model` precedence, and derives the model cover env from that provider's model list

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

### Requirement: Router mode env translation

In `Router` mode the spawn SHALL translate the router's `listen` address
and first `auth_token` into `ANTHROPIC_BASE_URL=http://{listen}` +
`ANTHROPIC_AUTH_TOKEN={token}` — the router always presents the Anthropic
protocol face to the agent. An empty `listen` is a resolution error; an
empty `auth_token` proceeds with a warning.

#### Scenario: router env construction

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

### Requirement: Preset data follows the code table

The built-in preset table SHALL remain hardcoded in the application code and SHALL be
read-only data that is never rewritten by any surface. A provider that selects a preset
(by name or explicit `preset` field) SHALL resolve its base URLs from that table at
load/resolve time — never from a persisted copy — so an application update that changes a
preset's URLs automatically applies to every provider derived from it without any
stored-data change. An explicit base URL on a preset-derived provider SHALL be a
configuration error, not a silent overwrite of preset data.

The provider's **model list is core-owned data, not preset data**. It is seeded from the
preset's code-table list when the provider is created, and after that the operator may add,
remove, or reorder models through the ordinary provider-management surface. The code table
is therefore a starting point rather than the authority: the same preset can back providers
with different model lists, and a preset's own list is never modified by a provider edit.

#### Scenario: derived provider resolves from code

- **WHEN** a provider entry names preset `deepseek` with no URL or models fields of its own
- **THEN** it resolves with the `deepseek` preset's base URLs and models from the built-in
  table

#### Scenario: code preset update propagates

- **WHEN** the application updates preset `kimi`'s URLs and the user restarts (or
  hot-reloads) without touching stored provider data
- **THEN** the preset-derived provider `kimi` serves the new URLs

#### Scenario: URL override rejected

- **WHEN** a preset-derived provider entry sets any explicit base URL slot
- **THEN** configuration parsing fails with an error explaining that preset-derived
  providers follow the code table

#### Scenario: api key stays user-owned

- **WHEN** the user stores an API key on a preset-derived provider and the application later
  ships a changed preset
- **THEN** the stored key is untouched by the preset update

#### Scenario: model list is editable per provider

- **WHEN** the operator adds a model to a provider derived from preset `deepseek`
- **THEN** that provider's stored model list contains it while a second provider on the same
  preset keeps its own list

#### Scenario: editing a provider never rewrites the preset table

- **WHEN** the operator edits the model list of a provider derived from preset `deepseek`
- **THEN** the code table's `deepseek` entry is unchanged, and a provider created later from
  the same preset starts from the code-table list again

### Requirement: Set default provider and model from the page

The provider management page SHALL let the operator mark a provider as the
default for new sessions and pick that provider's default model, and SHALL
show which provider and model are currently the default. Setting the default
SHALL persist across restarts (router-side, alongside the provider store) and
SHALL NOT alter any existing session's model. Clearing the default SHALL
return new sessions to the execution body's built-in default.

#### Scenario: set default from the page

- **WHEN** the operator marks provider `glm` with model `m2` as the default
- **THEN** the page shows `glm` / `m2` as the current default, and the value
  survives a router restart

#### Scenario: existing sessions are untouched

- **WHEN** the default changes while a session runs with its own selected
  model
- **THEN** that session keeps its selected model for subsequent turns

#### Scenario: clear the default

- **WHEN** the operator clears the default
- **THEN** new sessions use their execution body's built-in default and the
  page shows no default

### Requirement: Core owns provider and model data

The provider and model store SHALL be owned by the core process: core SHALL be its only
writer, and the feishu `/provider` card, the WebUI provider surface, and any
operator-facing management path SHALL obtain and mutate provider, model, alias, and
default data through core. The router SHALL be a read-only consumer of that data — it
resolves and serves them, and SHALL NOT persist, create, rename, delete, or override them
by any path, including a file fallback. Default provider and default model SHALL be
stored with the provider data, not in a separate process-owned file.

#### Scenario: card edit is written by core

- **WHEN** the `/provider` card stores a provider
- **THEN** the write is committed by core, and a later read from the card, the WebUI, and the router all observe the same value

#### Scenario: router cannot write provider data

- **WHEN** a management operation is attempted against the router process while no core is reachable
- **THEN** it fails honestly with an unavailable-source error and no provider file is written or modified

#### Scenario: defaults live with provider data

- **WHEN** the operator sets the default provider and model
- **THEN** core stores them alongside provider data, and they survive both a router restart and a WebUI restart without a separate defaults file

#### Scenario: router reads a core change without a restart

- **WHEN** core commits a provider change while the router is running
- **THEN** the router picks it up through its subscription and routes by the new configuration without a restart

### Requirement: Model entries carry capability tags

A provider's model list SHALL be a list of entries rather than a list of bare strings.
Each entry SHALL carry the model id and that model's capability tags. `text` SHALL be
implicit for every entry and SHALL NOT be stored; `vision`, `audio`, and `video` SHALL be
explicit and selectable, marking a model that accepts image, audio, or video input
respectively. Tags SHALL be per entry and editable by the operator. Legacy stored lists of
bare strings SHALL be accepted on read and normalised to entries carrying `text` alone, so
existing data keeps working without an offline migration. Capability tags SHALL be
metadata: they SHALL NOT alter routing, protocol selection, or whether a request is
accepted.

#### Scenario: an entry carries its id and tags

- **WHEN** a provider's model list is read
- **THEN** each element is an entry exposing its model id and its capability tags, with
  `text` implied on every entry

#### Scenario: multimodal tags are stored explicitly

- **WHEN** the operator marks a model as accepting image input
- **THEN** that entry carries `vision` in addition to the implicit `text`, and the other
  tags stay absent

#### Scenario: legacy string lists are accepted

- **WHEN** a provider's stored model list is still a list of bare strings
- **THEN** it is read as entries whose ids are those strings, each carrying `text` alone,
  with no error and no offline migration step

#### Scenario: tags do not gate requests

- **WHEN** a request targets a model whose tags omit `vision`
- **THEN** the request is routed and forwarded exactly as it would be without any
  capability tag
