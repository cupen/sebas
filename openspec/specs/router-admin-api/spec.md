# router-admin-api Specification

## Purpose
router 的管理 HTTP 面：webui 及其他管理客户端通过同端口 `/admin/*` 编辑 providers 与模型别名、探测上游 model 列表、触发配置热生效并读取运行状态；管理与透传两条流量的鉴权相互独立。

## Requirements

### Requirement: Admin authentication

Admin endpoints (`/admin/*` and `GET /metrics`) SHALL authenticate independently
of the LLM proxy traffic. When the `SEBAS_CONTROL_SECRET` environment variable
is set (watchdog deployment), requests MUST present
`Authorization: Bearer <secret>`; a missing or wrong token yields 401 with a
generic message that never echoes the presented value. When no secret is set
(standalone `sebas router`), admin endpoints MUST accept only loopback client
addresses and reject others with 401; startup logs a warning in that mode.
`/healthz` and proxy-traffic authentication semantics are unchanged.

#### Scenario: bearer accepted

- **WHEN** `SEBAS_CONTROL_SECRET` is set and a request to `/admin/providers`
  carries the correct bearer token
- **THEN** the request is processed normally

#### Scenario: wrong bearer rejected

- **WHEN** a request to `/admin/providers` carries a wrong or missing bearer
  token while a control secret is set
- **THEN** the response is 401 and the body does not contain the presented
  token value

#### Scenario: loopback fallback in standalone mode

- **WHEN** no control secret is set and a loopback client requests
  `/admin/providers`
- **THEN** the request is processed, while the same request from a
  non-loopback address yields 401

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

### Requirement: Write-then-apply semantics

A successful admin mutation SHALL be durably persisted before the response is returned, as a single committed transaction in the core state store, so concurrent readers never observe partial content, and by preserving stored data the router does not own (e.g. alias rows when mutating providers). A mutation whose parsed content is invalid MUST be rejected with 400 before any store write. After a successful mutation, the new configuration MUST be effective for requests that arrive after the response, without a process restart; requests already in flight continue under the configuration they started with.

#### Scenario: edit takes effect immediately

- **WHEN** `PUT /admin/providers/alpha` changes the OpenAI base URL and the next request arrives after the response
- **THEN** that request is forwarded to the new base URL

#### Scenario: invalid mutation rejected before write

- **WHEN** `POST /admin/providers` submits a provider with no base URL for either protocol and no preset
- **THEN** the response is 400, the state store is unchanged, and the running configuration is unchanged

### Requirement: Configuration source

The router SHALL read provider overrides (providers, deletions, model aliases) through the core channel state methods, backed by the core state store, merged on top of the config seed. The state store is the single source of truth for provider data, written by core on behalf of both the feishu `/provider` card path and this admin API. When no stored provider data exists, the config seed alone applies (unchanged behavior). Legacy JSON files SHALL NOT be imported: the state store starts empty.

#### Scenario: card-edited provider reaches router

- **WHEN** the `/provider` card flow stores provider `beta` with an OpenAI base URL and the router loads (or receives a change notification)
- **THEN** `beta` is routable through the router without editing config.toml

#### Scenario: seeded-only machine unchanged

- **WHEN** the state store contains no provider rows
- **THEN** the config seed alone applies

#### Scenario: migrated state file data moves back

- **WHEN** a machine upgrades with providers stored only in the legacy JSON files (state.json / providers.json)
- **THEN** the state store starts empty (no legacy import), and providers are re-created through the `/provider` card or admin API before they route again

### Requirement: External change hot reload

The router SHALL receive provider and model alias change notifications via its core channel subscription and apply the new configuration without a restart; a burst of commits MAY be coalesced so multiple notifications result in one reload. When the channel is unreachable or a notification is invalid, the router MUST keep serving the last valid configuration, log the failure, and expose the state through the admin surface; recovery happens automatically on the next valid change or reconnect. `POST /admin/reload` re-fetches configuration via the state methods on demand and reports success or the error.

#### Scenario: card edit hot-applies

- **WHEN** the feishu `/provider` card flow commits a new provider while the router is running
- **THEN** within a short coalescing window the provider becomes routable with no restart

#### Scenario: channel failure keeps serving

- **WHEN** the core channel is down
- **THEN** the router keeps routing with the last valid configuration and `/admin/stats` reports the state source as unavailable

#### Scenario: corrupt external write keeps serving

- **WHEN** a change notification cannot be applied (invalid content) or the state source reports an error
- **THEN** the router keeps routing with the last valid configuration and `/admin/stats` reports the error

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

### Requirement: Model alias CRUD endpoints

The admin API SHALL expose alias management:
`GET /admin/model-aliases` (list), `POST /admin/model-aliases` (create),
`PUT /admin/model-aliases/{alias}` (update),
`DELETE /admin/model-aliases/{alias}` (delete). Validation failures
(unknown provider, empty alias, alias containing `/`, duplicate alias) yield
400 or 409 respectively before any file write; unknown alias on update or
delete yields 404. Alias routing semantics are specified by the
router-model-aliases capability.

#### Scenario: alias create validates provider

- **WHEN** `POST /admin/model-aliases` references a provider that does not
  exist
- **THEN** the response is 400 and no file is written

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
