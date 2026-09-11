## ADDED Requirements

### Requirement: Model list fetch over the channel

core SHALL expose a model-list fetch on the provider state surface: given a provider
name, it performs one read-only upstream `GET` against that provider's resolved base
URL — `{base_url_openai_chat}/models` when set, else `{base_url_openai_responses}/models`,
else `{base_url_anthropic}/v1/models` — authenticated with the provider's resolved key,
and returns the discovered model id list. The fetch SHALL work for preset-derived and
custom providers alike, resolving a preset's base URLs from the code table. It SHALL
modify no provider field and SHALL persist nothing. A provider with no usable base URL
SHALL be a typed rejection naming the reason; an upstream failure SHALL be a typed
rejection carrying only a sanitized status or category, never key material or upstream
body content. The call SHALL be bounded by a timeout and SHALL NOT retry in a storm.

#### Scenario: fetch returns the upstream model ids

- **WHEN** a client requests a model-list fetch for a provider whose chat-completions slot serves a model list
- **THEN** the response carries the discovered ids and no key material

#### Scenario: preset-derived provider is fetchable

- **WHEN** a client requests a model-list fetch for a provider derived from a preset
- **THEN** the fetch uses the preset's code-table base URL and succeeds without any stored URL on that provider

#### Scenario: fetch persists nothing

- **WHEN** a model-list fetch succeeds
- **THEN** the provider's stored data is byte-for-byte unchanged, and only a later explicit edit may write a model list

#### Scenario: upstream failure is typed and sanitized

- **WHEN** the upstream returns an error status
- **THEN** the client receives a typed rejection naming the status or category, with no key material and no upstream body echoed

#### Scenario: provider without a usable base URL

- **WHEN** a client requests a model-list fetch for a provider with no base URL slot
- **THEN** the response is a typed rejection naming that reason, and no upstream call is made
