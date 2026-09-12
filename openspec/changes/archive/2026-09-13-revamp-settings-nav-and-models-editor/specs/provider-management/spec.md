## MODIFIED Requirements

### Requirement: Model probing

The probe-models action SHALL be available for providers with at least one usable base
URL slot, for preset-derived providers (using the preset's code-table base URLs) as
well as custom ones. Probing is performed by core on behalf of the requesting surface:
it issues a single `GET` (5 s timeout) to one URL — `{base_url_openai_chat}/models`
when set, else `{base_url_openai_responses}/models`, else
`{base_url_anthropic}/v1/models` — authenticating with the provider's resolved key
(plain-text key preferred, else the `api_key_env` value), and parses the `data[].id`
array. The upstream response carries model ids only: context window and similar
parameters SHALL be resolved from the local static table by id, and an unknown id
SHALL fall back to the documented default and be labelled unknown rather than guessed
from its name. Any error message SHALL be sanitized of key material.

For the WebUI settings surface, the probe entry SHALL live inside the provider editor
and its result SHALL replace the editor's in-memory model list wholesale (deduplicated
by id; existing entries whose ids survive the fetch keep their manually assigned
capability tags); the replacement is persisted only when the operator saves the
editor, and cancelling the editor discards the fetched list. The fetch itself writes
no provider field and, for preset-derived providers, leaves the preset's code-table
data unchanged. The IM `/provider` card surface keeps its result-card presentation:
picking a fetched id on a card writes the catalog and default model for custom
providers.

#### Scenario: single-URL probe choice

- **WHEN** a provider has both `base_url_openai_chat` and
  `base_url_anthropic` set and the user clicks probe
- **THEN** only the OpenAI chat `/models` endpoint is queried; there is no
  fallback attempt against the Anthropic URL on failure

#### Scenario: catalog writeback and default selection

- **WHEN** the probe against a custom provider returns models `m1` and
  `m2` and the user clicks `使用 m2` on the IM result card
- **THEN** the provider's model list contains `m1` and `m2` and its
  `default_model` is set to `m2`

#### Scenario: webui editor draft replacement

- **WHEN** the operator opens the WebUI editor for a custom provider whose model
  list contains `m1` (tagged `vision`) and runs probe, and the upstream returns
  `m1`, `m2`
- **THEN** the editor draft lists `m1` (still tagged `vision`) and `m2` (implicit
  text only), and the stored provider is unchanged until the editor is saved

#### Scenario: preset-derived probe is report-only

- **WHEN** the probe succeeds against a preset-derived provider
- **THEN** the WebUI editor draft (or IM result card) lists the models and can set
  the default model, but the fetch writes no provider field and the preset's
  code-table data is unchanged

#### Scenario: probe without openai URL

- **WHEN** a provider defines only `base_url_anthropic`
- **THEN** the probe targets `{base_url_anthropic}/v1/models`（Anthropic 槽位同样是
  usable base URL，单 URL 选择规则覆盖该形态）

#### Scenario: probe without usable base URL

- **WHEN** a provider defines no base URL in any slot
- **THEN** the WebUI editor renders no probe entry for it

#### Scenario: upstream parameters are not invented

- **WHEN** the probe returns a model id absent from the local static table
- **THEN** the result entry carries the id with its parameters marked unknown and
  the documented default applied, rather than a value derived from the model's name

#### Scenario: key material never reaches the result

- **WHEN** the probe succeeds or fails
- **THEN** neither the result presentation nor the error text contains the
  provider's key or the `api_key_env` value
