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

### Requirement: Configuration source

The router SHALL read provider overrides (providers, deletions, model aliases) through
the core channel state methods, backed by the core state store, merged on top of the
config seed. The core state store SHALL be the single source of truth for provider and
model data, and core SHALL be its only writer: the feishu `/provider` card path, the
WebUI provider surface, and this process's admin read views all obtain and mutate that
data through core. The router SHALL NOT write provider, alias, or default data by any
path — no admin mutation endpoint, no file fallback, no direct write to
`providers.json` or a defaults file. When no stored provider data exists, the config seed
alone applies. Legacy JSON files SHALL NOT be imported and SHALL NOT be written: the
state store starts empty and remains the only authority.

#### Scenario: card-edited provider reaches router

- **WHEN** the `/provider` card flow stores provider `beta` with an OpenAI base URL and the router loads (or receives a change notification)
- **THEN** `beta` is routable through the router without editing config.toml

#### Scenario: seeded-only machine unchanged

- **WHEN** the state store contains no provider rows
- **THEN** the config seed alone applies

#### Scenario: migrated state file data moves back

- **WHEN** a machine upgrades with providers stored only in the legacy JSON files (state.json / providers.json)
- **THEN** the state store starts empty (no legacy import), and providers are re-created through the `/provider` card or the WebUI provider surface before they route again

#### Scenario: router has no provider write path

- **WHEN** a client sends a POST, PUT, or DELETE against the router's provider, model-alias, or defaults admin routes
- **THEN** no such route exists (404), the store is unchanged, and no provider or defaults file is written

#### Scenario: router without a reachable core writes nothing

- **WHEN** the router runs with no core reachable and a management operation is attempted against it
- **THEN** the operation fails with an unavailable-source error and the on-disk provider data is left untouched

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
