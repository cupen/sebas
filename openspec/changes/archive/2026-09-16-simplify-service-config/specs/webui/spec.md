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
the workspace root — the listing starts at the workspace root, and an
explicit `root` query parameter is honoured only inside it), `POST
/api/sessions/{key}/archive` (archive a session), `POST
/api/sessions/{key}/restore` (restore an archived session), `GET /api/archive`
(list archived sessions with expiry info), and `GET /ws` (WebSocket session
stream). Project and session mutations are POST-only and carry the same
posture as the existing session APIs. The provider management cluster under
`/api/*` (`GET/POST /api/providers`, `PUT/DELETE /api/providers/{name}`,
`POST /api/providers/{name}/probe`, `GET /api/provider-presets`, `GET
/api/provider-defaults`, `GET/POST/DELETE /api/model-aliases`, `DELETE
/api/model-aliases/{alias}`) SHALL be fulfilled by the WebUI backend from the
core-owned provider store over the core channel, never by proxying the router
process. The retired `/router/api/*` namespace (including `POST
/router/api/reload`) and the retired `GET /api/router` endpoint SHALL NOT be
served. Without a reachable core these routes SHALL fail honestly (503) and
SHALL NOT serve a stale snapshot. The JSON admin API `/api/admin/*` (status,
events, services, update, update/dry-run, update/dev, rollback, restart) is
always mounted: without a control-plane adapter its reads report
`adapter_ok: false` and its mutations return 503 (honest degradation). `GET
/health` returns the literal `ok`. All browser assets the UI needs to render
— styles, fonts, Web Awesome, markdown rendering, and syntax highlighting —
are self-hosted under `/assets/*`; the UI SHALL NOT depend on an external CDN
at render time. Navigation SHALL only link to routes this surface serves.

The WebUI SHALL enforce the workspace root (per the `workspace-root`
capability) on every project-directory surface: local project registration
SHALL accept only paths inside the workspace root; the project list SHALL
omit local projects outside it; and viewing or opening a session bound to a
local project outside it SHALL be rejected. Housekeeping writes on such
sessions (close, archive) SHALL remain available so out-of-scope sessions can
be cleaned up. A legacy `[watchdog.webui] allowed_roots` key SHALL be
ignored.

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
- **THEN** the listing is rooted at the workspace root (which replaces the
  former work-directory start) and the response `path` echoes its canonical
  form

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

- **WHEN** the workspace root is `/home/op/work`
- **AND** `GET /api/fs/browse-dirs` is called with `root=/etc`
- **THEN** the response is 400 with an out-of-scope error, and no directory
  content outside the workspace root is disclosed

#### Scenario: browse-dirs accepts an allowed root

- **WHEN** a request carries the workspace root itself, or a subpath of it,
  as the explicit `root` (or as `path`)
- **THEN** the listing succeeds for that scope

#### Scenario: unconfigured allowed_roots preserves current behavior

- **WHEN** neither `SEBAS_WORKSPACE_ROOT` nor the config entry provides a
  workspace root
- **THEN** the process working directory becomes the root and a startup
  warning recommends explicit configuration — the former "no constraint when
  unconfigured" behavior is replaced by an always-present boundary

#### Scenario: project registration rejects a path outside allowed roots

- **WHEN** `POST /api/projects` is called with a body path that resolves
  outside the workspace root
- **THEN** the response is 400 with an out-of-scope error and the project is
  not registered

#### Scenario: project registration accepts a path inside allowed roots

- **WHEN** `POST /api/projects` is called with a path inside the workspace
  root
- **THEN** the project registers as before

#### Scenario: project registration without allowed_roots is unchanged

- **WHEN** the config still carries a legacy `[watchdog.webui] allowed_roots`
  key
- **THEN** the key is ignored at parse time and registration is governed
  solely by the workspace root — the whitelist mechanism no longer exists

#### Scenario: out-of-root project is hidden from the project list

- **WHEN** a project registered before the workspace root was tightened
  resolves outside the workspace root
- **AND** the browser requests `GET /api/projects`
- **THEN** that project is not listed

#### Scenario: opening a session of an out-of-root project is rejected

- **WHEN** a session is bound to a local project directory outside the
  workspace root
- **AND** the browser requests the session detail, sends it a message, or
  switches to it
- **THEN** the response is a 4xx out-of-scope rejection, and no spawn or turn
  is started

#### Scenario: housekeeping on an out-of-root session stays available

- **WHEN** a session is bound to a local project directory outside the
  workspace root
- **AND** the browser closes or archives that session
- **THEN** the request succeeds so the out-of-scope session can be cleaned up

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

### Requirement: Local-only binding

The standalone WebUI SHALL default to a loopback bind (`127.0.0.1:9797`). The legacy `core --webui` path binds hard-coded `127.0.0.1`. A non-loopback `service.webui.host` SHALL be refused with a configuration error unless the authentication switch is enabled and at least one enabled user exists in the user store (see「鉴权开关（auth）与首启用户引导」and「非 loopback bind 与 开关联动」below).

#### Scenario: non-loopback refused without auth

- **WHEN** the config sets `service.webui.host = "0.0.0.0"` while the user
store holds no enabled user, or while `auth = false`
- **THEN** `sebas webui` exits with a configuration error rather than
binding

### Requirement: 非 loopback bind 与开关联动

当 `service.webui.host` 非 loopback 时，webui SHALL 仅在 `auth = true` （或缺省）且用户库存在至少一个启用用户时才允许绑定启动。零用户时 SHALL 先经环境变量引导（`SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD`） 建立 root，否则以配置错误拒绝启动——防止公网下「先访问者注册 root」。开关关闭时无论用户库为何，SHALL 拒绝非 loopback bind （防止误关开关叠加公网暴露）。

#### Scenario: 开关关闭拒绝公网 bind

- **WHEN** 配置同时设置 `auth = false` 与 `host = "0.0.0.0"`
- **THEN** `sebas webui` 以配置错误退出，不绑定端口

#### Scenario: 开关打开但零用户拒绝公网 bind

- **WHEN** 配置设置 `auth = true`（或缺省）、`host = "0.0.0.0"`，
用户库零用户且未设凭据环境变量
- **THEN** `sebas webui` 以配置错误退出，不绑定端口

#### Scenario: 开关打开且凭据存在允许公网 bind

- **WHEN** 配置设置 `auth = true`（或缺省）、`host = "0.0.0.0"`，
且用户库存在启用用户（含环境变量引导建立的 root）
- **THEN** `sebas webui` 正常绑定并在日志中提示已启用登录鉴权

### Requirement: Watchdog lifecycle ownership

The WebUI SHALL be spawned by the watchdog as a separate process (with the control secret) by default — `[service.webui] enabled` defaults to `true` unless explicitly set to `false` — and SHALL survive core restarts. The WebUI SHALL bind to `127.0.0.1:9797` by default; port conflict with a legacy `core --webui` (or any other process) is resolved by kernel-level bind atomicity — the first to bind wins, the second bind fails with a distinct exit code.

#### Scenario: single owner

- **WHEN** the watchdog-spawned WebUI is running and a legacy
`core --webui` is attempted
- **THEN** the second start is refused by the ownership guard (port
already bound)

#### Scenario: default enablement

- **WHEN** the watchdog starts with a configuration that contains no
`[service.webui]` section
- **THEN** the watchdog spawns and supervises the WebUI child process

#### Scenario: explicit disable

- **WHEN** the configuration sets `[service.webui] enabled = false`
- **THEN** the watchdog does not spawn the WebUI and reports it as a
disabled service

### Requirement: 鉴权开关（auth）与首启用户引导

WebUI SHALL 提供 `[service.webui] auth` 配置开关，默认 `true`。开关为 `true` 时，鉴权门 SHALL 恒在：`/api/*`、`/router/api/*`、`/ws` 需要有效 会话；用户库（auth.db）零用户时 SHALL 不自动生成任何凭据，改为进入 首启引导流程（见 `webui-user-management` 能力：设置页或环境变量建立 root），期间 `GET /api/auth/me` SHALL 报告 `needs_setup: true`。开关为 `false` 时，无论用户库是否存在用户，SHALL 对所有路由（含静态资源） 完全放行，不要求登录且不触发引导；`GET /api/auth/me` SHALL 报告 `enabled: false`（前端据此不渲染登录页）。`sebas webui-passwd` 在开关 关闭时仍可管理用户（为重新启用做准备），但不产生任何强制登录效果。

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

- **WHEN** 配置设置 `service.webui.auth = false`
- **THEN** 未带任何会话的 `/api/summary` 请求返回 200，全部路由免登录
- **AND** `GET /api/auth/me` 返回 `{"enabled": false, "authenticated": false}`

#### Scenario: 关闭后重新打开立即生效

- **WHEN** 开关从 `false` 改回 `true` 并重启 webui
- **THEN** 用户库中已有的用户立即恢复强制登录，无需重建用户
