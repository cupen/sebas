## ADDED Requirements

### Requirement: Router-free API surface and IPC-only server edge

The WebUI's HTTP API surface SHALL NOT expose the router process as a concept:
no route namespace or endpoint named after the router, and no
WebUI-originated network connection to the router process. The WebUI
process's only server-side network edge is the core channel (IPC); provider,
model alias, and defaults data flows exclusively through the core-owned state
store over that channel, and configuration propagation to the router happens
solely via the router's own core channel subscription. LLM inference traffic
(agent execution bodies ↔ router ↔ upstream providers) SHALL NOT transit the
WebUI. Static deployment facts about the router parsed from configuration at
startup (listen address, provider count) are not a network edge and MAY
remain as display metadata.

#### Scenario: retired router namespace is gone

- **WHEN** any request hits a `/router/api/*` path or `GET /api/router`
- **THEN** the response is 404, and no such route is registered

#### Scenario: provider change propagates without a WebUI-to-router call

- **WHEN** the operator renames a provider through the WebUI provider surface
- **THEN** the mutation reaches the core state store over the core channel,
  the router reloads via its own core channel subscription, and no HTTP call
  from the WebUI to the router process occurs (the retired reload proxy no
  longer exists)

#### Scenario: reload trigger has no WebUI surface

- **WHEN** the operator wants router configuration to take effect immediately
- **THEN** no WebUI endpoint exists for that purpose; the trigger is the
  router's own `POST /admin/reload` (ops/CLI, out of the WebUI's scope) or an
  automatic core-channel notification

## MODIFIED Requirements

### Requirement: HTTP route surface

The WebUI SHALL serve `GET /` as the SPA shell for the project workbench and
`GET /assets/*` for its built styles, scripts, and fonts. Any other
browser-facing GET (for example `/sessions/{key}`) resolves through the SPA
fallback, and the retired IA-v1 paths `/settings`, `/gateway`, and `/about`
canonicalise to `/` — those surfaces live in the Settings modal now. The JSON
API SHALL serve: `GET /api/sessions` and `POST /api/sessions` (create, with
optional `prompt` field and a required `agent` field naming the target
agent), `GET /api/sessions/{key}`, `POST /api/sessions/{key}/message`,
`POST /api/sessions/{key}/cancel` (interrupt the session's in-flight turn
over the core channel), `POST /api/sessions/{key}/close`,
`POST /api/sessions/{key}/switch`,
`POST /api/sessions/{key}/pending/{pending_id}/remove` (remove a
not-yet-started submission), `POST /api/sessions/{key}/pending/{pending_id}/move`
(reorder a not-yet-started submission within its own disposition group; body
`to_index`), `GET /api/summary`, `POST /api/permissions/{request_id}/answer`,
`GET /api/settings`, `GET /api/about`, `POST /api/sessions/{key}/model`
(mid-session model switch), `POST /api/sessions/{key}/mode` (mid-session
permission-mode switch), the agent catalog `GET /api/agents` (each configured
agent plus the built-in native kernel, with id, display name, reachability,
optional cause and version), `GET /api/provider-presets` (read-only preset
table), `GET/POST /api/auth/login`, `GET /api/auth/me`, `POST
/api/auth/logout`, the project APIs `GET /api/projects` and `POST
/api/projects` (register), `POST /api/projects/reorder`, `POST
/api/projects/{id}/remove`, `GET /api/projects/{id}/branch`, `GET
/api/fs/browse-dirs` (lazy directory listing for the folder picker, scoped to
the server's work directory — the configured work dir of the default agent
kind, falling back to the WebUI process working directory; an explicit `root`
query parameter overrides the default), `POST /api/sessions/{key}/archive`
(archive a session), `POST /api/sessions/{key}/restore` (restore an archived
session), `GET /api/archive` (list archived sessions with expiry info), and
`GET /ws` (WebSocket session stream). Project and session mutations are
POST-only and carry the same posture as the existing session APIs. The
provider management cluster under `/api/*` (`GET/POST /api/providers`,
`PUT/DELETE /api/providers/{name}`, `POST /api/providers/{name}/probe`,
`GET /api/provider-presets`, `GET /api/provider-defaults`,
`GET/POST/DELETE /api/model-aliases`, `DELETE /api/model-aliases/{alias}`)
SHALL be fulfilled by the WebUI backend from the core-owned provider store
over the core channel, never by proxying the router process. The retired
`/router/api/*` namespace (including `POST /router/api/reload`) and the
retired `GET /api/router` endpoint SHALL NOT be served. Without a reachable
core these routes SHALL fail honestly (503) and SHALL NOT serve a stale
snapshot. The JSON admin API `/api/admin/*` (status, events, services,
update, update/dry-run, update/dev, rollback, restart) is always mounted:
without a control-plane adapter its reads report `adapter_ok: false` and its
mutations return 503 (honest degradation). `GET /health` returns the literal
`ok`. All browser assets the UI needs to render — styles, fonts, Web Awesome,
markdown rendering, and syntax highlighting — are self-hosted under
`/assets/*`; the UI SHALL NOT depend on an external CDN at render time.
Navigation SHALL only link to routes this surface serves.

`GET /api/fs/browse-dirs` SHALL honour a path round-trip contract: the `path`
echoed in a listing response SHALL be accepted verbatim as the `path` of a
subsequent request for that same directory, and request paths that mix `/`
and `\` separators SHALL resolve to the same directory. The echoed path SHALL
NOT carry a Windows verbatim (`\\?\`) prefix.

Projects SHALL be identified on the wire by a stable `project_id`
(`proj-<12hex>`, deterministically derived from the canonicalised path), not
by the raw path string; the path itself SHALL NOT appear in session or
project API request/response bodies as an identifier, though it may be
included as display metadata.

The session payloads the workbench observes — the focused session in `GET
/api/summary` and `GET /api/sessions/{key}` — SHALL carry the session's
pending submissions in delivery order, each with its stable id, text,
position, disposition (`staging` | `turn`) and priority flag.

#### Scenario: dashboard route

- **WHEN** a browser requests `/`
- **THEN** the SPA workbench renders, listing registered projects in the
  project rail, the History (archive) group, and the selected project's
  sessions

#### Scenario: session deep link still resolves

- **WHEN** a bookmarked `/sessions/{key}` is requested
- **THEN** the SPA fallback serves the shell and the client router renders
  the session's detail rather than 404, so links made before this change keep
  working

#### Scenario: admin cluster requires adapter

- **WHEN** the WebUI starts without a control-plane adapter
- **THEN** `/api/admin/*` reads report `adapter_ok: false` and mutations
  return 503

#### Scenario: router data reflects live state

- **WHEN** a provider is renamed through the provider management surface and
  the browser then requests `GET /api/providers`
- **THEN** the response lists the new provider name without a WebUI restart

#### Scenario: router mutations unavailable without secret

- **WHEN** the WebUI has no reachable core and a mutation is posted to
  `/api/providers`
- **THEN** the response is 503

#### Scenario: retired router surface is gone

- **WHEN** any request hits `/router/api/*` or `GET /api/router`
- **THEN** the response is 404

#### Scenario: agents catalog lists configured agents and native

- **WHEN** the browser requests `GET /api/agents`
- **THEN** the response lists one entry per configured agent (`id` = the
  config key, `display` = its display name) plus one entry for the native
  kernel (`id = "native"`), each with `reachable`, an optional `cause` when
  unreachable, and an optional `version`; no `driver` or
  backend-implementation field appears in the payload

#### Scenario: create session requires an explicit agent

- **WHEN** `POST /api/sessions` is called without an `agent` field, or with a
  legacy `backend` value (`"acp"`, `"acp:<slug>"`, or null)
- **THEN** the request is rejected with 400 naming the `agent` field as
  required, and no session is created

#### Scenario: project id is the wire identifier

- **WHEN** a session is created with `project_id = "proj-abc123def456"` and
  later listed
- **THEN** the session and project payloads reference the project by that
  `project_id`, not by the raw directory path

#### Scenario: agent defaults read and set

- **WHEN** the browser requests `GET /api/agent-defaults` or `PUT
  /api/agent-defaults`
- **THEN** the response is 404 — this endpoint no longer exists. Default
  provider and model are managed through the provider management surface
  (`/api/providers`, `/api/provider-defaults`), and the default agent is
  remembered per project, not through a global agent-defaults endpoint.

#### Scenario: no external asset fetch

- **WHEN** any page is rendered
- **THEN** every stylesheet, script, and font it requests resolves under
  `/assets/*` and no request targets an external host

#### Scenario: navigation targets exist

- **WHEN** every navigation link in the rendered shell is requested
- **THEN** each resolves to a route served by this surface, including SPA
  client routes resolved through the fallback

#### Scenario: browse-dirs defaults to the server work directory

- **WHEN** `GET /api/fs/browse-dirs` is called with no `root` parameter
- **THEN** the listing is rooted at the server's configured work directory
  (or the process working directory when none is configured), and the
  response `path` echoes that directory

#### Scenario: browse-dirs path round-trips on expand

- **WHEN** the `path` echoed by a directory listing is sent back unchanged as
  the `path` of a request for one of its subdirectories (client-side join of
  the echoed parent and a child name)
- **THEN** the response is 200 with that subdirectory's entries, including on
  Windows where canonicalised paths would otherwise carry a `\\?\` prefix

#### Scenario: directory browser rejects parent escape

- **WHEN** `GET /api/fs/browse-dirs?path=/etc&root=/home/user` is called
- **AND** the path resolves outside the scoped root
- **THEN** the response is 400 with an error message

#### Scenario: archive endpoint exists

- **WHEN** `POST /api/sessions/{key}/archive` is called
- **THEN** the session is archived, moved from the active session list, and
  the response confirms the action

#### Scenario: archive list endpoint

- **WHEN** `GET /api/archive` is called
- **THEN** the response lists all archived sessions with their original
  project, archive timestamp, and expiry

#### Scenario: browse-dirs rejects a root outside the allowed list

- **WHEN** `[watchdog.webui] allowed_roots` is configured as `["~/work"]`
- **AND** `GET /api/fs/browse-dirs` is called with `root=/etc`
- **THEN** the response is 400 with an out-of-scope error, and no directory
  content outside the allowed roots is disclosed

#### Scenario: browse-dirs accepts an allowed root

- **WHEN** `allowed_roots` contains a directory and a request carries that
  directory as the explicit `root` (or a subpath of it as `path`)
- **THEN** the listing succeeds for that scope

#### Scenario: unconfigured allowed_roots preserves current behavior

- **WHEN** `allowed_roots` is absent or empty
- **AND** `GET /api/fs/browse-dirs` is called with an explicit `root` that
  exists
- **THEN** the listing succeeds — the whitelist is opt-in, and the default
  fallback root (work dir / process cwd) remains the no-`root` scope

#### Scenario: project registration rejects a path outside allowed roots

- **WHEN** `allowed_roots` is configured
- **AND** `POST /api/projects` is called with a body path that resolves
  outside every allowed root
- **THEN** the response is 400 with an out-of-scope error and the project is
  not registered

#### Scenario: project registration accepts a path inside allowed roots

- **WHEN** `allowed_roots` is configured
- **AND** `POST /api/projects` is called with a path inside an allowed root
- **THEN** the project registers as before

#### Scenario: project registration without allowed_roots is unchanged

- **WHEN** `allowed_roots` is absent or empty
- **AND** `POST /api/projects` is called with an existing directory path
- **THEN** the project registers as before (existence checks only)

#### Scenario: pending submissions ride in the session payload

- **WHEN** the browser requests a session payload while submissions are
  staged during spawn or queued behind a running turn
- **THEN** the payload lists them in delivery order with id, text, position,
  disposition and priority

#### Scenario: pending submission removal and reorder are served

- **WHEN** `POST /api/sessions/{key}/pending/{pending_id}/remove` or
  `.../move` is called for a submission that has not started
- **THEN** the request succeeds and a subsequent session payload reflects the
  new pending list

#### Scenario: managing a started submission is a typed rejection

- **WHEN** either pending endpoint is called for a submission whose turn
  already started
- **THEN** the response is a 4xx typed rejection stating that it is already
  running, and the in-flight turn is unaffected

#### Scenario: queue overflow is a visible rejection

- **WHEN** `POST /api/sessions/{key}/message` is called while the session's
  spawn-window staging queue is at its cap
- **THEN** the response is a 4xx rejection naming the cap, and the response
  does not report the submission as accepted

#### Scenario: cancel forwards to the core channel

- **WHEN** `POST /api/sessions/{key}/cancel` is called for an existing
  session and the core is reachable
- **THEN** the WebUI forwards the cancel request over the core channel and
  returns the outcome

#### Scenario: cancel without the core is honest

- **WHEN** `POST /api/sessions/{key}/cancel` is called while the core is
  unreachable
- **THEN** the response is 503 with the degradation cause, and no success is
  reported

### Requirement: Mutation posture

Admin mutations SHALL be POST-only (non-POST gets 405) and guarded by an
origin check: empty origin or loopback origin (`127.0.0.1`, `localhost`,
`::1`) passes; a non-loopback origin is rejected with 403. Provider
management mutation routes under `/api/providers*` and
`/api/model-aliases*` follow the same posture: POST-only with the same origin
check, and they apply to the core-owned provider store over the core channel
— no control secret and no router admin API are involved server-side. In the
shipped UI, browser buttons post with a loopback origin, which is the
operative authentication path for mutations.

#### Scenario: post-only

- **WHEN** a GET hits `/admin/restart`
- **THEN** the response is 405

#### Scenario: foreign origin rejected

- **WHEN** a mutation POST carries `Origin: https://evil.example`
- **THEN** the response is 403

#### Scenario: router mutation is post-only and origin-checked

- **WHEN** a GET hits `/api/providers` or a provider mutation POST carries a
  non-loopback origin
- **THEN** the response is 405 (GET) or 403 (foreign origin)

Access to `/api/admin/*` — authentication and role authorization — SHALL be
governed solely by the「鉴权开关（auth）与首启用户引导」requirement and the
RBAC permission table (`services.control`) defined in the
webui-user-management capability; the retired per-control-plane password
session imposes no additional gate. Honest degradation without a
control-plane adapter is unchanged.

#### Scenario: control plane needs only the workbench session

- **WHEN** an authenticated `admin`-role session calls `POST
  /api/admin/restart`
- **THEN** the mutation proceeds under `services.control` authorization — no
  second control-plane login or CSRF token is required

### Requirement: Provider management page

The WebUI settings SHALL provide a provider management page backed by the
core-owned provider store (see `provider-management`), reached through the
WebUI's `/api/providers` surface. It SHALL support: listing providers with
name, preset-or-custom mark, base URL slots, key-configured state, and each
model entry's capability tags; creating a provider either from a preset or as
custom; editing an existing provider; and deleting a provider. Fetching a
provider's official model list is specified separately by the `Fetch models
from the provider's official base URL` requirement, not here.

The model list SHALL be a list of entries the operator may add to and remove
from freely, each entry carrying an id and its capability tags (`text`
implicit, with `vision`, `audio`, and `video` selectable). The list SHALL be
editable for preset-derived and custom providers alike.

Creating from a preset SHALL require only the preset choice, the API key, and
optionally the model entries; the preset's base URLs SHALL render as
read-only code-owned values. Creating a custom provider SHALL require the
same inputs plus an instance name, a base URL, and a wire protocol. Every
input beyond that minimum — the remaining base URL slots, the model rename
map, and the preset's inherited `api_key_env` — SHALL live in an Advanced
disclosure that is collapsed by default. `api_key_env` SHALL NOT be a user
input field; a preset's env name remains only a fallback key source when no
plaintext key is stored.

Secret inputs SHALL never be pre-filled, and an empty secret submit SHALL
preserve the stored key. Mutations SHALL surface their errors (duplicate,
validation, unavailable) in the page. Choosing an option inside any select
control of the provider editor SHALL NOT close the editor.

#### Scenario: preset-derived provider shows read-only URLs

- **WHEN** the user opens provider `glm` (preset-derived) for editing
- **THEN** the base URL fields render the preset's code-owned values as
  read-only with a follow-the-code indication, while the API key, default
  model, model entries, and their capability tags remain editable

#### Scenario: create from preset

- **WHEN** the user creates a provider from preset `glm` entering an API key
  and two model entries
- **THEN** the create request carries the preset selection, the key, and the
  model entries with capability tags, but no base URL fields, and the
  provider appears in the list

#### Scenario: custom provider full editing

- **WHEN** the user creates a custom provider filling a single base URL
- **THEN** the create succeeds and the edit form exposes all three slots —
  the two extras under the Advanced disclosure — for later adjustment

#### Scenario: probe from the page

- **WHEN** the user runs a fetch on a provider with a usable base URL
- **THEN** the page shows the returned model ids without leaving the page

#### Scenario: delete reflects immediately

- **WHEN** the user deletes provider `alpha` from the page
- **THEN** the provider disappears from the list without a page reload

#### Scenario: preset instance name defaults without input

- **WHEN** the user creates from preset `glm` without opening the Advanced
  disclosure
- **THEN** the provider is stored under the name `glm`, and renaming it is
  the only path to a second instance of the same preset

#### Scenario: custom provider minimal input

- **WHEN** the user creates a custom provider entering an instance name, a
  base URL, and a wire protocol
- **THEN** the create succeeds without the operator opening the remaining URL
  slots or the model rename map

#### Scenario: advanced section collapsed by default

- **WHEN** the provider editor opens in either mode
- **THEN** the remaining base URL slots, the model rename map, and the
  inherited `api_key_env` are behind a collapsed disclosure, and none of them
  is submitted as a user-entered value unless the operator opens it

#### Scenario: multiple model entries with capability tags

- **WHEN** the user adds several model entries to a provider and marks one as
  `vision`
- **THEN** the list preserves order, that entry carries `vision` alongside
  the implicit `text`, and the others carry `text` alone

#### Scenario: model list is editable for a preset-derived provider

- **WHEN** the user adds a model entry to a preset-derived provider
- **THEN** the entry is stored on that provider without altering the preset's
  code-table data

#### Scenario: selecting an option does not close the editor

- **WHEN** the user picks a preset, a protocol, or a model option inside the
  provider editor
- **THEN** the editor stays open and the chosen value is retained

#### Scenario: secret is preserved on empty submit

- **WHEN** the user edits a provider and submits with the API key field left
  empty
- **THEN** the stored key is unchanged and no key material was rendered

### Requirement: 鉴权开关（auth）与首启用户引导

WebUI SHALL 提供 `[watchdog.webui] auth` 配置开关，默认 `true`。开关为
`true` 时，鉴权门 SHALL 恒在：`/api/*`、`/ws` 需要有效会话；用户库
（auth.db）零用户时 SHALL 不自动生成任何凭据，改为进入首启引导流程（见
`webui-user-management` 能力：设置页或环境变量建立 root），期间 `GET
/api/auth/me` SHALL 报告 `needs_setup: true`。开关为 `false` 时，无论用户库
是否存在用户，SHALL 对所有路由（含静态资源）完全放行，不要求登录且不触发
引导；`GET /api/auth/me` SHALL 报告 `enabled: false`（前端据此不渲染登录
页）。`sebas webui-passwd` 在开关关闭时仍可管理用户（为重新启用做准
备），但不产生任何强制登录效果。

#### Scenario: 默认打开且有用户

- **WHEN** 配置未写 `auth` 且用户库存在启用用户
- **THEN** 未带会话的 `/api/summary` 请求返回 401，行为与无开关时一致

#### Scenario: 默认打开且零用户进入设置流程

- **WHEN** 配置未写 `auth`、用户库零用户、且未设凭据环境变量
- **THEN** `GET /api/auth/me` 返回 `needs_setup: true`（前端渲染首启
  设置页而非登录页），受保护 API 仍对未带会话请求返回 401

#### Scenario: 环境变量引导 root

- **WHEN** 用户库零用户且 `SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD`
  非空
- **THEN** 启动时建立名为该用户名的 root 用户，`needs_setup` 不再出现

#### Scenario: 测试环境关闭

- **WHEN** 配置设置 `watchdog.webui.auth = false`
- **THEN** 未带任何会话的 `/api/summary` 请求返回 200，全部路由免登录
- **AND** `GET /api/auth/me` 返回
  `{"enabled": false, "authenticated": false}`

#### Scenario: 关闭后重新打开立即生效

- **WHEN** 开关从 `false` 改回 `true` 并重启 webui
- **THEN** 用户库中已有的用户立即恢复强制登录，无需重建用户
