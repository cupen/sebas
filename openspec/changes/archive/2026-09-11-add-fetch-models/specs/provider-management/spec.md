## MODIFIED Requirements

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
