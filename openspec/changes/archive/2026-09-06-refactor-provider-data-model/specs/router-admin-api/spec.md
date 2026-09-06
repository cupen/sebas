## MODIFIED Requirements

### Requirement: Provider CRUD endpoints

The admin API SHALL expose provider management:
`GET /admin/providers` (list), `POST /admin/providers` (create),
`PUT /admin/providers/{name}` (update), `DELETE /admin/providers/{name}`
(delete). List and single views MUST include name, the bound preset name
(`preset`, absent for custom providers), the three base URL
slots (`base_url_anthropic`, `base_url_openai_chat`,
`base_url_openai_responses`), `api_key_env`, the model list, and an
`api_key_configured` boolean — never the key material itself. Create and
update payloads accept the same three slots; a custom provider with none
of them is a 400. An update that omits the api_key field or submits it
empty MUST preserve the stored key. Creating a provider whose name already
exists yields 409; updating or deleting an unknown name yields 404.
Deleting a provider that exists in the config seed MUST persist across
restarts (tombstone).

#### Scenario: list masks keys

- **WHEN** `GET /admin/providers` returns provider `alpha` which has an API
  key configured
- **THEN** the response contains `api_key_configured: true` and no key
  material anywhere in the body

#### Scenario: list carries three slots

- **WHEN** `GET /admin/providers` returns a provider with all three slots
  configured
- **THEN** each slot appears under its own field name, and the removed
  `base_url_openai` field appears nowhere in the response

#### Scenario: url field on preset-derived provider rejected

- **WHEN** `POST /admin/providers` submits an entry bound to a preset and
  carrying an explicit base URL slot
- **THEN** the response is 400 naming the field, and no file is written

#### Scenario: empty key submit preserves key

- **WHEN** `PUT /admin/providers/alpha` is called with the api_key field
  empty
- **THEN** the stored API key is unchanged

#### Scenario: duplicate create rejected

- **WHEN** `POST /admin/providers` names a provider that already exists
- **THEN** the response is 409 and no file is written

### Requirement: Model probe endpoint

`POST /admin/providers/{name}/probe` SHALL query the upstream model list by
trying the OpenAI-compatible `/models` endpoint first and falling back to
the Anthropic `/v1/models` endpoint, authenticated with the provider's
resolved key. The OpenAI attempt uses the chat-completions slot when set,
else the Responses slot. The response returns the discovered model list;
with `?apply=true` the list is persisted into that provider's model list
field — custom providers only; a preset-derived provider persists nothing
(its catalog follows the code table). A provider without any configured
base URL yields a 400 with a reason; upstream failures yield 502 with a
generic message that never includes the key.

#### Scenario: probe returns models

- **WHEN** `POST /admin/providers/alpha/probe` runs against a provider
  whose chat-completions slot serves a model list
- **THEN** the response contains the model list and no key material

#### Scenario: probe apply persists

- **WHEN** the probe runs with `?apply=true` against a custom provider and
  succeeds
- **THEN** the provider's model list in the overlay file is replaced with
  the probed list

#### Scenario: probe apply skips preset-derived provider

- **WHEN** the probe runs with `?apply=true` against a preset-derived
  provider and succeeds
- **THEN** the response contains the model list but the stored provider
  data gains no models catalog

## ADDED Requirements

### Requirement: Preset table endpoint

`GET /admin/presets` SHALL return the built-in preset table as read-only
data: for each preset its name, the configured base URL slots, and the
models catalog. The data served MUST come from the running binary's code
table (never a stored copy), so clients rendering preset-derived providers
stay in sync with the code. The endpoint SHALL NOT accept mutations — the
preset table is code-owned.

#### Scenario: presets reflect the running binary

- **WHEN** `GET /admin/presets` is served by a binary containing an updated
  preset URL
- **THEN** the response carries the updated URL even if stored provider
  data predates it

#### Scenario: presets are read-only

- **WHEN** a mutation is attempted against the preset table
- **THEN** no such endpoint exists; the router serves only the read view
