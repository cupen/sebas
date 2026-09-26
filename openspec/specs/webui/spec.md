# webui Specification

## Purpose
Defines the local web dashboard: its HTTP route surface, the local-only
security baseline (loopback binding, optional admin password, mutation
guards), the session dashboard and its focus/close semantics, the detached
behavior of the watchdog-spawned instance, and the admin actions proxied to
the watchdog control plane.

## Requirements

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
permission-mode switch), `POST /api/sessions/{key}/activate` (start the
child in the background for a placeholder/dormant session), `GET
/api/sessions/{key}/approvals` (approval read model for restoring pending
review cards), `POST /api/sessions/{key}/label` (set the operator label),
the agent catalog `GET /api/agents` (each configured
agent plus the built-in native kernel, with id, display name, reachability,
optional cause and version), `GET /api/nodes` (remote execution node list),
the skills surface `GET/POST /api/skills` (store listing; sync trigger) and
`GET/DELETE /api/skills/{name}`, `GET /api/provider-presets` (read-only preset
table), `POST /api/auth/login` (the login page itself is rendered by the SPA),
`GET /api/auth/me`, `POST
/api/auth/logout`, the project APIs `GET /api/projects` and `POST
/api/projects` (register), `POST /api/projects/reorder`, `POST
/api/projects/{id}/remove`, `GET /api/projects/{id}/branch`, `GET
/api/fs/browse-dirs` (lazy directory listing for the folder picker, scoped to
the workspace root — the listing starts at the workspace root, and an
explicit `root` query parameter is honoured only inside it), `POST
/api/sessions/{key}/archive` (archive a session), `POST
/api/sessions/{key}/restore` (restore an archived session), `GET /api/archive`
(list archived sessions with expiry info), `GET /api/archive/{key}` (one
archived session's snapshot), and `GET /ws` (WebSocket session
stream). Project and session mutations are POST-only and carry the same
posture as the existing session APIs. The provider management cluster under
`/api/*` (`GET/POST /api/providers`, `PUT/DELETE /api/providers/{name}`,
`POST /api/providers/{name}/probe`, `GET /api/provider-defaults`, `POST
/api/model-aliases` (create/update one alias), `PUT/DELETE
/api/model-aliases/{alias}`; the alias table is read through the providers
projection rather than a dedicated list endpoint) SHALL be fulfilled by the WebUI backend from the
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

### Requirement: 降级与错误表现

The WebUI frontend SHALL surface a visible global indicator when its live
connection to the server (`/ws`) is lost, and SHALL clear that indicator and
refresh visible view data automatically when the connection is restored
(the existing refetch hook). Data requests that fail at the network level
(server process down, DNS/connection failure) SHALL be distinguishable from
server-side business errors (4xx/5xx with a backend error body) so views can
react appropriately. List-style views (dashboard, project rail, sessions)
SHALL render an inline failure state with a retry affordance instead of a
blank panel when their initial data load fails. The workbench composer's
submit gate SHALL consume the same WS-pushed core reachability state as the
global banner (initial get + flip notifications): a reported
`reachability.ok = false` SHALL disable the composer until a recovery
notification or a fresh get response reports reachable, and `/api/summary`
SHALL remain a pure on-demand read endpoint whose availability no longer
feeds the submit gate.

#### Scenario: ws 断线显示全局横幅

- **WHEN** the `/ws` connection drops
- **THEN** a global connection banner is visible in the app shell, and the
  existing exponential-backoff reconnect keeps running

#### Scenario: 重连恢复后横幅消失并刷新数据

- **WHEN** the `/ws` connection is re-established
- **THEN** the banner disappears and the visible views refetch their data
  (the existing `sebas:refetch` behavior)

#### Scenario: 网络级失败可区分于业务错误

- **WHEN** a data request fails without an HTTP response (server process is
  down, connection refused)
- **THEN** the frontend error is a recognizable network-unreachable error
  rather than a backend-`ApiError` with business semantics

#### Scenario: 列表加载失败显示内联重试态

- **WHEN** the initial data load of the dashboard, project rail, or sessions
  view fails
- **THEN** the view renders an inline failure state with a retry affordance
  instead of an empty or silently stale panel

#### Scenario: summary 轮询失败等同 core 不可达

- **WHEN** the 5s polling is retired and the composer's reachability source
  is WS push only, while `/api/summary` remains available as an on-demand
  read endpoint
- **THEN** the submit gate follows only pushed `core.reachability` state
  (initial get + flip notifications); an `/api/summary` failure on the
  on-demand path SHALL no longer disable or enable the submit gate

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

### Requirement: Session dashboard and focus semantics

The cross-project session list SHALL render one row per known session (encoded key, operator label or first-message preview, session id, status slug, relative last-active), active-first, and SHALL be reachable from the workbench rather than from primary navigation. The session list SHALL exclude archived sessions — those are served by `GET /api/archive`. Selecting a session in the rail, opening its `/sessions/{key}` deep link, or posting `/switch` SHALL focus that session in place — a display pointer only that never changes message routing — and `switch` returns the redirect target or 404 for an unknown key. There SHALL be no separate per-session detail surface: the workbench renders the focused session. Switching the displayed project SHALL NOT alter the focused session pointer. The rail's current-session marker SHALL be derived from the focused-session pointer, not from the browser location.

The focused-session pointer SHALL be the single source of truth for the workbench composer's follow-up vs creation mode: with a focused session the composer targets that session; without one the composer is in creation mode. Any path that focuses a session (switch endpoint, deep-link visit, placeholder creation) SHALL leave the composer able to submit a follow-up message to that session without further operator action.

`GET /api/summary` SHALL NOT embed the focused session's full transcript; conversation content SHALL be fetched via the per-session detail endpoint with an incremental cursor. The dashboard SHALL rate-limit and dispatch refetches: lightweight events (session metadata, presence) refresh lists, turn content events update only the focused conversation via its cursor — an operator's browser SHALL NOT issue a full refetch storm (multi-request × whole-transcript responses) per streamed frame.

前端 SHALL 在会话重建或 core 重启后作废本地增量游标：当快照响应携带的世代/起始位置与本地游标矛盾（本地游标大于服务端当前日志长度，或会话标识世代变化）时，客户端 SHALL 丢弃本地游标与缓冲、重取全量快照，不得因陈旧游标永久拒收增量。

对话视图 SHALL 在回合进行中自动跟随流式输出：当操作者已聚焦该会话并处于贴底跟随状态时，新到内容 SHALL 持续滚动可见；未读缝的显隐 SHALL NOT 重建整个对话 DOM（既有条目的展开态 SHALL 保留）。流式期间渲染 SHALL NOT 对未变化的历史条目做整块重解析——正文增量 SHALL 以增量方式合并进当前条目。

#### Scenario: focus is cosmetic

- **WHEN** the user focuses session B in the WebUI while session A is active
- **THEN** subsequent Feishu messages still route per the core's own session mapping, unchanged

#### Scenario: switch unknown key

- **WHEN** `/api/sessions/{key}/switch` posts a key not in the map
- **THEN** the response is 404

#### Scenario: rail selection focuses in place

- **WHEN** the operator selects a session in the rail
- **THEN** that session becomes focused, the workbench renders its conversation in place, and the operator is not navigated to a separate detail page

#### Scenario: project switch leaves focus alone

- **WHEN** the operator switches the displayed project
- **THEN** the focused session pointer is unchanged

#### Scenario: archive hides session from list

- **WHEN** a session is archived
- **THEN** it is no longer returned by `GET /api/sessions` and appears only in the `GET /api/archive` response

#### Scenario: focusing enables immediate follow-up

- **WHEN** a session becomes focused through any supported path
- **THEN** the workbench composer's next submission is delivered to that session as a follow-up message

#### Scenario: summary stays small while transcript is large

- **WHEN** the focused session has a multi-megabyte transcript and a turn is streaming
- **THEN** `GET /api/summary` responses remain in the kilobyte range; conversation content flows only through the session detail endpoint with the cursor

#### Scenario: streamed frame does not trigger full refetch

- **WHEN** a `turn.append` frame arrives
- **THEN** the dashboard updates the focused conversation from the frame (or a cursor-limited detail fetch), without re-fetching nodes, projects, sessions, and the full summary

#### Scenario: stale cursor after core restart converges

- **WHEN** the core restarts and the rebuilt session log assigns positions from zero while the browser holds an old high-water cursor
- **THEN** the browser detects the contradiction, resets its cursor, refetches the full snapshot, and subsequent increments apply normally

#### Scenario: auto-scroll follows streaming at the bottom

- **WHEN** the operator is focused and pinned to the bottom while output streams
- **THEN** new content stays in view without manual scrolling

#### Scenario: seam toggle preserves DOM state

- **WHEN** the unread seam appears or disappears during streaming
- **THEN** previously rendered entries are not rebuilt from scratch and expanded thinking/tool items keep their open state

### Requirement: Web session close

`POST /api/sessions/{key}/close` SHALL kill the ACP child when the mapping
is active, drop the mapping and card state, clear the chat-level permission
allowlist and reply target, and clear the focused-session pointer if it
pointed at the closed session. Dormant mappings drop without a kill. Unknown
keys return 404. For an active session the workbench asks for confirmation
inline (the rail row's overflow menu) before sending the close; the close
endpoint itself performs no server-side confirmation.

#### Scenario: close active session

- **WHEN** the user confirms closing an active session from the workbench
- **THEN** the child process is terminated, the mapping is removed, and the
  permission allowlist for that chat is cleared

#### Scenario: close unknown

- **WHEN** the close endpoint is called with an unknown key
- **THEN** the response is 404 and nothing is mutated

### Requirement: Standalone core-client semantics

The watchdog-spawned (standalone) WebUI SHALL obtain session data and perform
session mutations exclusively through the core session channel, and SHALL NOT
construct its own `DispatchHandle`, restore session state from the state file, or
hold a throwaway session manager. Session create, message send, and close SHALL
be requests to the core that spawn real ACP sessions and take effect in the
running core. The in-process `core --webui` path SHALL use an equivalent
in-process backend so that both paths present the same behavior to the browser.

#### Scenario: standalone message send reaches the core

- **WHEN** the user sends a message through a standalone WebUI's session page
- **THEN** the request is delivered to the core, which applies it to the real
  session, and the change is observable in the core rather than only in the
  WebUI process

#### Scenario: standalone board is live

- **WHEN** the core creates or updates a session while a standalone WebUI page is
  open
- **THEN** the WebUI reflects the change without a manual reload, and never
  renders a session set reconstructed from the state file

#### Scenario: both paths behave alike

- **WHEN** the same page is rendered under `sebas webui` and under
  `core --webui`
- **THEN** session data and the availability of session controls are equivalent,
  differing only in which backend implementation serves them

### Requirement: Session backend seam
The WebUI crate SHALL access sessions through a backend abstraction rather than a concrete `DispatchHandle`, in the same shape as the existing admin adapter, so the crate carries no knowledge of whether the core is in-process or across a socket. The crate SHALL NOT depend on the sebas binary crate to obtain a backend; the binary crate SHALL supply the implementation at startup. **补充**：`SessionBackend` 实现 SHALL 在 `reachability()` 中区分 startup failure / auth rejected / runtime disconnect 三类不可达，并在 `/api/summary` 输出中通过 `reachability.kind` 字段显式区分；webui degradation banner SHALL 根据 kind 渲染不同文案（startup failure banner 含 startup-failure cause；runtime disconnect banner 不含）。**补充**：approval_answer 与 set_session_model SHALL 在 webui 端到端走 fake-claude 沙箱验证（happy-path + typed-rejection 路径）；approval_answer 流程 SHALL 经真实审批通道（acp gated tool call → channel ApprovalRequested 帧 → webui review-card → POST `/api/permissions/{rid}/answer` → channel ApprovalAnswer → acp 子进程以 allow/deny 语义继续）。

#### Scenario: WebUI is testable without a core

- **WHEN** the WebUI's route tests run
- **THEN** they drive routes through a fake backend, with no ACP child, no socket,
  and no state file

#### Scenario: no backend leaks into templates

- **WHEN** a page is rendered
- **THEN** which backend is in use is not visible in the markup except where the
  channel's degradation contract requires stating that the core is not connected

#### Scenario: startup-failure banner via fake-claude sandbox

- **WHEN** 沙箱 backend core 启动时配置错误退出 75（`SEBAS_STARTUP_ERROR_FILE=<tmp>` 写入 `startup-failure: bad config`）、webui 仍连接该 socket
- **THEN** webui banner SHALL 显示 `core startup failed: bad config` 全串（cause 即该全串，与已落地的 enrich 行为一致）；`GET /api/summary.reachability.kind` SHALL 为 `startup_failed`

#### Scenario: runtime disconnect banner distinguishes from startup failure

- **WHEN** 沙箱 backend core 启动后正常运行；测试期间强杀 core 进程；webui 探测
- **THEN** banner 显示 `core is not connected`（不含 startup failure 字样）；`/api/summary.reachability.kind` SHALL 为 `disconnected`

#### Scenario: approval_answer end-to-end (allow path)

- **WHEN** fake-claude 触发 gated tool call（触发词 "perm"）、core 推到 channel ApprovalRequested 帧、webui 渲染 review-card、操作员点 allow
- **THEN** webui POST `/api/permissions/{rid}/answer` with allow；core 转发 ApprovalAnswer 到 acp 子进程；子进程以 allowed 工具结果呈现；transcript 完成（acp 回合 Done）

#### Scenario: approval_answer end-to-end (deny path)

- **WHEN** fake-claude 触发 gated tool call、操作员点 deny
- **THEN** webui POST `/api/permissions/{rid}/answer` with deny；core 转发 ApprovalAnswer 到 acp 子进程；子进程以 denied 工具结果呈现；回合 Done

#### Scenario: approval_answer end-to-end (detached topology)

- **WHEN** 同一流程跑在双进程沙箱（独立 core + 独立 webui，harden 5.4 的可复用 harness）：fake-claude 触发 gated tool call → channel ApprovalRequested 帧跨进程到达 webui → review-card → answer → ApprovalAnswer 回 core → acp 子进程继续
- **THEN** allow / deny 各一条旅程全绿；单进程形态的同名用例保持全绿（两种拓扑不互相代替）

#### Scenario: approval_answer rejects unknown request_id

- **WHEN** webui POST `/api/permissions/{rid}/answer` with `rid` 不存在
- **THEN** 后端回 4xx typed rejection（`UnknownRequestId` 或同类）；前端不渲染成功状态

#### Scenario: set_session_model happy-path via webui

- **WHEN** 沙箱 fake-claude 启动带 `configOptions.model` 多 model、用户 POST `/api/sessions/{key}/model` with `{model_id: "<另一 model>"}`
- **THEN** 后端走 channel SetSessionModel；acp 子进程回 `ModelChanged` 事件；webui snapshot 同步 `current_model`；前端 UI 显示新 model selected

#### Scenario: set_session_model rejects unknown model

- **WHEN** 用户 POST `/api/sessions/{key}/model` with `{model_id: "<不存在的 model>"}`
- **THEN** 后端走 channel SetSessionModel；acp 子进程回 `Error`（typed rejection）；webui 端呈现内联错误；session state 不变

#### Scenario: cross_uid rejection covers live process

- **WHEN** live fork + setuid 到不同账户进程尝试连接 core socket 并发 Snapshot 请求
- **THEN** 连接被拒；服务端日志写 peer-uid mismatch；不进入 Snapshot 处理路径

#### Scenario: backend push renders faithfully

- **WHEN** backend 推送 spawn-failure 事件
- **THEN** 前端立即渲染该事件为 transcript 内显错误，不等待 Removed 事件；会话状态 SHALL 在 backend 推送 Removed 之前已经标记为 spawn-failed

### Requirement: Admin actions via control plane
Admin mutations SHALL proxy to the watchdog over the control RPC using the `SEBAS_CONTROL_SECRET`, attributed to a local CLI actor — which executes directly without a confirmation round-trip. **补充**：Actions 在原有 update (release)、update dry-run、update dev、rollback、restart core 之外，新增「Services 分区内的 enable/disable/restart」三类——enable/disable 经 `POST /api/admin/services/{name}/enable|disable` 走 `ServiceSet` RPC（仅对辅助服务 webui/router/im 开放；core 恒启动，无 enable/disable 入口）；restart per-process 经既有 watchdog 监督循环（每个受管服务都有 restart 入口）。**补充**：裸 core 形态下所有 admin mutations SHALL 返回 503 "control plane not connected"，Services 分区 SHALL 据此呈现退化。

#### Scenario: restart via admin

- **WHEN** the admin clicks restart on `/admin/update`-style pages with the control secret present
- **THEN** the watchdog receives `RestartCore` and restarts the core; the WebUI itself stays up

#### Scenario: no control plane

- **WHEN** the standalone WebUI has no `SEBAS_CONTROL_SECRET`
- **THEN** admin mutation buttons return 503; Services 分区显示「无 watchdog 控制面」横幅

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

### Requirement: WebUI bind failure exit code

The WebUI child process SHALL exit with a reserved exit code
(`EXIT_BIND_FAILED = 75`) when it fails to bind to the configured
address, so the watchdog supervisor can distinguish bind failures from
other crashes. The supervisor SHALL recognize this code, log a warning
naming the service, and mark the WebUI service as `Degraded` instead of
retrying.

#### Scenario: port already occupied

- **WHEN** the watchdog starts and `127.0.0.1:9797` is already bound by
  another process
- **THEN** the WebUI child exits with code 75, the supervisor logs a
  warning naming the WebUI service, and reports the state as `Degraded`

#### Scenario: recovery via restart

- **WHEN** the WebUI is `Degraded` due to a port conflict, the blocking
  process exits, and a control-plane request restarts the WebUI service
- **THEN** the WebUI binds successfully, the supervisor reports `Running`

#### Scenario: non-bind crash is not degraded

- **WHEN** the WebUI child exits with a code other than 75
- **THEN** the supervisor treats it as a normal crash and retries with
  backoff

### Requirement: Supervisor Degraded state

The `ServiceState` enum SHALL have a `Degraded` variant. When a service
enters `Degraded`, the supervisor SHALL stop spawning and wait for either
a `Restart` or `Stop` command. A `Restart` command SHALL reset the
service back to `Restarting` and attempt a new spawn.

#### Scenario: degraded service does not auto-retry

- **WHEN** a service is in `Degraded` state
- **THEN** the supervisor does not call `spawn()` again until a `Restart`
  command is received

#### Scenario: restart clears degraded

- **WHEN** a degraded service receives a `Restart` command
- **THEN** the supervisor sets the state to `Restarting` and calls
  `spawn()`

### Requirement: Archive persistence

The archive registry SHALL persist to its own file, separate from the project registry and the core state store. Each entry SHALL record the session key, the original project path, the session label, the archive timestamp, and the retention deadline.

The archive file's location SHALL resolve in the following priority order: an explicit `SEBAS_ARCHIVE_PATH` override; otherwise `archive.json` in the state database's directory (the directory of `SEBAS_STATE_DB`), so that deployments which pin the state directory — sandboxes in particular — pin the archive with it. The legacy default (`archive.json` directly under the home `.sebas` directory) SHALL be honored as a migration source only: when the resolved location holds no archive file and the legacy location does, the WebUI SHALL move the legacy file to the resolved location at startup; when the move fails, the WebUI SHALL warn and continue reading from the legacy location rather than silently starting empty. A migration SHALL be announced through the WebUI's notification channel.

#### Scenario: archive survives restart

- **WHEN** the WebUI process is restarted after sessions were archived
- **THEN** the same archived sessions are listed

#### Scenario: archive expiry clean on startup

- **WHEN** the WebUI starts and an archived session has passed its retention deadline
- **THEN** that entry is removed from the archive file and the session is no longer listed

#### Scenario: pinned state directory pins the archive

- **WHEN** the WebUI runs with `SEBAS_STATE_DB` pointing inside a sandbox directory and no `SEBAS_ARCHIVE_PATH` is set
- **THEN** the archive file is read and written inside that same sandbox directory, and no path under the real home directory is read or written

#### Scenario: explicit override wins

- **WHEN** `SEBAS_ARCHIVE_PATH` is set
- **THEN** that exact file is used regardless of the state database location

#### Scenario: legacy archive migrates forward

- **WHEN** the WebUI starts with no archive file at the resolved location and a legacy archive file exists under the home `.sebas` directory
- **THEN** the legacy file is moved to the resolved location, its entries are listed as before, and a notification announces the migration

#### Scenario: failed migration degrades read-only rather than empty

- **WHEN** the legacy move fails at startup
- **THEN** the WebUI warns, continues to serve the legacy entries, and does not start an empty archive over them

### Requirement: Focused session termination is reflected consistently

When a session the operator is focused on is terminated and removed by the
backend — child process crash, dispatch reaping, or any other removal — the
focused view SHALL leave the Working state in the same update cycle: it SHALL
NOT continue rendering a live session with an active stop control once the
backend no longer knows the session. The termination SHALL be announced
through the notification channel with the session label and the observed
cause (for a child crash, at minimum that the agent process exited
unexpectedly), and the transcript already received SHALL remain viewable. The
rail, the focused view, and the backend session list SHALL agree on the
session's existence.

#### Scenario: child crash ends the Working state

- **WHEN** the focused session's agent child crashes after emitting partial
  output and the backend removes the session
- **THEN** the focused view stops presenting Working and its stop control, a
  notification names the crashed session, the received transcript stays
  readable, and the rail no longer lists the session

#### Scenario: rail and focused view agree with the backend

- **WHEN** a session is removed by the backend for any reason
- **THEN** within the same update cycle the rail drops the row and a focused
  view of that session either closes or presents the read-only remnant — it
  never shows a live state the backend does not confirm

### Requirement: Configurable archive retention

The WebUI config SHALL support an `archive_retention_days` field under the `[service.webui]` section (the WebUI 配置的现唯一归属地；无独立 `[webui]` 顶层节), with a default of 30 days. The expiry check SHALL run at WebUI startup and on every `GET /api/archive` or `GET /api/sessions` request.

#### Scenario: default retention

- **WHEN** no `archive_retention_days` is set in the config
- **THEN** the default retention of 30 days applies

#### Scenario: custom retention

- **WHEN** `[service.webui] archive_retention_days = 60` is set
- **THEN** archived sessions are retained for 60 days

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

### Requirement: Honest session rejection causes

会话创建/驱动被拒时呈现给操作员的原因 SHALL 区分"核心（通道）不可达"与
"目标执行体不可用"：执行体侧的拒绝（如 native 缺 provider 凭据）SHALL 在
文案中指名执行体与真实原因，SHALL NOT 复用"核心不可达"（unreachable）字样；
仅当请求确实无法送达会话权威（通道断开、核心不在）时才呈现不可达语义。

#### Scenario: native 缺凭据的拒绝不再误报核心不可达

- **WHEN** 核心在运行且通道连通，但 native 执行体未配置 provider 凭据，
  客户端以 `backend: "native"` 请求创建会话
- **THEN** 拒绝文案指名 native 执行体与缺凭据原因，且不包含"核心不可达"

#### Scenario: 通道断开仍呈现不可达

- **WHEN** 核心未运行（通道不可达）时请求创建会话
- **THEN** 拒绝呈现核心不可达语义及其 cause

### Requirement: web_spawn 失败的立即内显
webui 在为会话派生 acp 子进程（web_spawn）时 SHALL 把 spawn failure 立即 inline 到 transcript 作为可读错误事件，**不**延后到后续 Removed 事件或会话状态变更。错误事件 SHALL 包含失败原因（exit code / stderr 末行 / spawn error message）与时间戳；transcript SHALL 在新子进程的未成功期间持续呈现该错误而不被后续成功 turn 隐式覆盖。Removed 事件仍可作为次要信号补充，但 SHALL NOT 作为 spawn failure 的首次呈现路径。

#### Scenario: spawn failure surfaces immediately in transcript

- **WHEN** webui 为某会话派生 acp 子进程失败（exit code 非零或 spawn 系统调用失败）
- **THEN** transcript SHALL 立即出现一条错误事件，含失败原因；会话状态 SHALL 标记为 spawn-failed（而非仍按 placeholder/empty 假装存在）

#### Scenario: repeated failure does not silently retry

- **WHEN** 同一会话连续 web_spawn 失败 2 次以上
- **THEN** transcript SHALL 累计呈现失败计数与最近一次原因，**不**静默重试；操作员 SHALL 在 UI 上明确看到 spawn 已失败

#### Scenario: spawn success after prior failure clears error

- **WHEN** 一次失败的 spawn 之后用户重试并成功派生
- **THEN** 新 turn 正常进入 transcript；之前的 spawn-failed 错误事件保留为历史（不删除），但会话状态恢复为非 spawn-failed

### Requirement: 设置弹窗分区与缺省首项
设置弹窗 SHALL 暴露分区导航，分区 SHALL 按下列顺序排列：`Generic` → `Appearance` →（组间分隔线）`Services` → `Models` →（弹性留白 + 组间分隔线，压底）`Env Vars` · `About`。打开弹窗时缺省聚焦 `Generic` 分区；用户上次停留分区 SHALL 在新会话首次打开时被记住（localStorage），之后打开仍按记忆回到上次分区；记忆中的值若已不存在于分区表（如旧值 `settings`），SHALL 回退到缺省分区。

导航项 SHALL 提供足够的点击目标与选中可见性：行高约 36px、字号不低于 0.875rem、整行 hover 反馈、当前项以左侧 accent 竖条标示。

`Generic` 分区 SHALL 收敛为纯通用可配置项分区，不再承载环境变量表；在语言切换等偏好落地前，主区 SHALL 呈现说明占位文案（指明偏好项后续提供）。

`Env Vars` 分区 SHALL 承载环境变量只读表（数据来自 `GET /api/env`，见「环境变量只读展示」），与 `About` 同属底部只读参考组。

`About` 分区 SHALL 分两段呈现：INSTANCE 段在上（工作区根目录 + 复制按钮、当前 default agent kind——读自运行时数据而非写死字面量；不呈现 default provider/model 行——创建预选不再依赖配置默认，见 agent-workbench「Model selector offers the backend catalog before any session」），BUILD 段在下（`/api/about` 的运行时构建信息）。

原 `Settings` 总览分区移除，其维护动作「全部进程重启」与「重置 Settings」SHALL 一并移除——逐服务 restart 由 Services 分区承载，不做广播式入口。

#### Scenario: 缺省聚焦 Generic 分区

- **WHEN** 操作员从侧栏底部打开设置弹窗，且无历史记忆
- **THEN** 弹窗打开后左导航高亮 `Generic`，主区渲染 `Generic` 分区内容（占位文案）

#### Scenario: 历史记忆恢复上次分区

- **WHEN** 操作员上次停留在 `Services` 后关闭弹窗，再打开
- **THEN** 弹窗缺省聚焦 `Services` 分区

#### Scenario: 缺省聚焦 Settings 分区

- **WHEN** localStorage 记忆值为 `settings`（本变更前的合法分区名，`Settings` 总览分区已由 `Generic` 接替）
- **THEN** 左导航高亮 `Generic` 而非报错或空白

#### Scenario: 分区顺序与底部只读组

- **WHEN** 设置弹窗渲染左导航
- **THEN** 分区按 `Generic → Appearance → Services → Models → Env Vars · About` 排列，`Appearance` 与 `Services` 之间有组间分隔线；`Env Vars` 与 `About` 通过弹性留白压在导航底部、上方有分隔线，与功能区视觉分离

#### Scenario: 分区顺序与 About 压底

- **WHEN** 设置弹窗渲染左导航
- **THEN** 功能分区按 `Generic → Appearance → Services → Models` 排列，`About` 与 `Env Vars` 同处底部只读组、整体通过弹性留白压底且上方有分隔线

#### Scenario: Settings 分区总览

- **WHEN** 聚焦 `About` 分区
- **THEN** 主区先呈现 INSTANCE 段——工作区根目录（路径 + 复制按钮）、default agent kind（只读，取真实运行时值），后呈现 BUILD 段（版本、commit、构建时间）；INSTANCE 段不含 default provider/model 行

#### Scenario: Generic 分区不再有环境变量表

- **WHEN** 聚焦 `Generic` 分区
- **THEN** 主区呈现偏好占位文案，环境变量只读表不再出现在该分区

#### Scenario: 环境变量表移居 Env Vars 分区

- **WHEN** 聚焦 `Env Vars` 分区
- **THEN** 主区渲染环境变量只读表（名字、解释、按分类展示的值），数据来自 `GET /api/env`

#### Scenario: About 分区承载实例信息

- **WHEN** 聚焦 `About` 分区
- **THEN** 主区先呈现 INSTANCE 段（工作区根目录 + 复制按钮、default agent kind 只读真实值），后呈现 BUILD 段（版本、commit、构建时间）

#### Scenario: About 不再呈现 default provider/model

- **WHEN** 操作员聚焦 `About` 分区，无论当前是否配置了默认 provider/model
- **THEN** INSTANCE 段不渲染 default provider/model 行，也不渲染跳转 Models 分区的对应链接

#### Scenario: 高危动作二次确认

- **WHEN** 设置弹窗任意分区渲染
- **THEN** 不存在「全部进程重启」与「重置 Settings」入口——原高危动作已随 `Settings` 总览分区删除，确认对话框随之消失；逐服务 restart 只在 Services 分区出现

### Requirement: 全局核心可达性横幅

app-shell SHALL 提供全局「核心不可达」fatal 通知，其状态由 WS 推送驱动而非轮询：客户端在 `/ws` 连接建立或重连后 SHALL 主动请求当前可达性（`core.reachability.get`），此后随 `core.reachability` 翻转通知即时更新；WS 断线窗口内错过的翻转 SHALL 由重连后的 get 响应收敛。当推送的可达性状态为 `reachability.ok = false` 时，在视口顶部居中通知层呈现驻留横幅，并对工作台整体施加锁定遮罩（交互与浏览一并锁住，工作台内的交互元素不可聚焦、不可激活）；横幅文案 SHALL 按 `reachability.kind` 分档呈现（`startup_failed` / `auth_rejected` / `disconnected` 各成一句），并保留 cause 原文；横幅呈现 role=alert 且 SHALL NOT 可手动关闭。锁定期间可达性推送订阅 SHALL 保持；横幅自身 SHALL 可交互；auth 门禁页（登录 / 首启设置）SHALL 不受锁定影响。可达性恢复通知到达后横幅 SHALL 消失、锁定 SHALL 解除，并 SHALL 弹出一条「核心已恢复」的 info 级通知。

#### Scenario: core 不可达时横幅出现

- **WHEN** core 进程停止或通道不可达（翻转通知到达），浏览器停留在工作台任意页面
- **THEN** fatal 横幅即时出现且文本按 `reachability.kind` 分档并含 cause 原文，工作台被锁定遮罩覆盖、其中交互元素不可聚焦不可激活

#### Scenario: core 恢复后横幅消失

- **WHEN** core 恢复服务且通道重新握手成功
- **THEN** 恢复通知到达后无需刷新页面，横幅即时消失、锁定解除，并出现「核心已恢复」info 通知

#### Scenario: 连接建立即获当前态

- **WHEN** 浏览器建立（或断线重连后重建）`/ws` 连接
- **THEN** 客户端发起可达性 get 请求并以响应初始化横幅与锁定状态，不依赖轮询

#### Scenario: 断线窗口的翻转由重连收敛

- **WHEN** `/ws` 断线期间 core 经历不可达→可达翻转
- **THEN** 重连后的 get 响应携带当前真实状态，横幅与锁定据此收敛

#### Scenario: auth 门禁页不受锁定影响

- **WHEN** 鉴权启用且操作者处于登录页或首启设置页时 core 不可达
- **THEN** 登录 / 设置流程照常可交互，不出现工作台锁定遮罩

### Requirement: Directory listing omits system directories

`GET /api/fs/browse-dirs` SHALL NOT list child directories that resolve onto the built-in system-directory denylist. The filter is a coarse UX filter for the picker tree; authoritative enforcement remains at project registration. Non-denylisted entries SHALL be unaffected, and the round-trip contract (an echoed `path` is accepted verbatim as a later request) SHALL continue to hold.

#### Scenario: denylisted child is not listed

- **WHEN** a directory listing would contain a child directory that resolves onto the built-in denylist (for example browsing a workspace root of `/`, which contains `/usr` and `/etc`)
- **THEN** those children are absent from the response entries

#### Scenario: other entries and the round-trip are unaffected

- **WHEN** a directory listing contains only non-denylisted subdirectories
- **THEN** all of them are listed as before, and joining a child name onto the echoed `path` resolves to that child on a subsequent request

### Requirement: Project registration rejects system directories

Local project registration (`POST /api/projects`) SHALL reject a submitted path that resolves onto a built-in system directory with a 400 error naming the submitted path (never the server-resolved form), and no project SHALL be created. The comparison SHALL run on the resolved real path — so `..` segments, path aliases, and symlinks pointing at a denylisted directory are caught — and SHALL be an exact match on the directory itself: a subdirectory of a denylisted directory remains registrable subject to the workspace-root containment rules. On Windows the comparison SHALL be case-insensitive, and drive roots (`C:\`, `D:\`, …) SHALL be matched by pattern rather than enumeration. The built-in denylist SHALL cover, on Unix: `/`, `/bin`, `/sbin`, `/boot`, `/dev`, `/etc`, `/lib`, `/lib32`, `/lib64`, `/libx32`, `/proc`, `/sys`, `/usr`, `/var`, `/run`, `/root`, `/home`, `/tmp` — and deliberately not `/opt`, `/srv`, `/mnt`, `/media`; on Windows: drive roots, `C:\Windows`, `C:\Program Files`, `C:\Program Files (x86)`, `C:\ProgramData`, `C:\Users`, `System Volume Information`, and `$Recycle.Bin`. The denylist is built-in and fixed; no configuration can extend or trim it. Both sides of the comparison SHALL be resolved before matching, so platform directory aliases (for example macOS `/tmp` → `/private/tmp`) hit the list. The denylist judgement SHALL run after the workspace-root containment judgement, which keeps its existing precedence and error.

#### Scenario: system directory itself is rejected

- **WHEN** the workspace root is `/` and `POST /api/projects` is called with path `/usr`
- **THEN** the response is 400 naming `/usr` as a system directory, and no project is registered

#### Scenario: a subdirectory of a denylisted directory stays registrable

- **WHEN** the workspace root is `/` and `POST /api/projects` is called with path `/home/user/code`
- **THEN** the project registers as before — only the denylisted directory itself is refused

#### Scenario: alias and traversal resolve before the comparison

- **WHEN** a submitted path reaches a denylisted directory through `..` segments, a filesystem alias, or a symlink inside the workspace root
- **THEN** the registration is rejected exactly as if the system directory had been named directly

#### Scenario: Windows comparison is case-insensitive and covers drive roots

- **WHEN** on Windows `POST /api/projects` is called with `c:\WINDOWS` or with a drive root such as `D:\`
- **THEN** the response is 400 naming the submitted path, and no project is registered

#### Scenario: out-of-scope precedence is unchanged

- **WHEN** a submitted path lies outside the workspace root and also happens to be a system directory
- **THEN** the out-of-scope rejection is returned — the containment judgement keeps precedence

### Requirement: Workspace root resolving onto a system directory warns at startup

When the resolved workspace root is itself a built-in system directory, the assembling process SHALL log a startup warning naming the resolved root and SHALL start normally — the registration denylist is the safety net, so a too-wide root degrades to denylist-enforced operation rather than misbehaving silently.

#### Scenario: root at a system directory starts with a warning

- **WHEN** the workspace root resolves to `/` (or another built-in system directory) and the WebUI assembles
- **THEN** a warning naming the resolved root is logged at startup, and the server starts and serves normally

### Requirement: 项目注册降级如实提示

`POST /api/projects` 在状态库（核心通道）不可用而落到本地文件注册表时，响应 SHALL 携带降级标记（含 cause 语义），前端 SHALL 就地提示"核心不可达，已写入本地注册表"一类的如实文案；状态库可用时响应不含该标记，前端无降级提示。两者皆失败时保持既有 503 行为。

#### Scenario: 核心不可达时加项目获得降级提示

- **WHEN** 核心通道不可达时通过 UI 注册一个合法项目目录
- **THEN** 注册成功（201）且响应携带降级标记，项目栏出现该项目并伴随降级提示，用户不再直到新建会话才得知核心不可达

#### Scenario: 核心正常时无降级提示

- **WHEN** 核心通道正常时注册项目
- **THEN** 响应无降级标记，UI 仅呈现常规成功路径

### Requirement: Creation-time model applies to ACP sessions

The `model` field of `POST /api/sessions` SHALL be honored for ACP-backend sessions: when present, the backend SHALL deliver it to the spawned ACP child as the session's model configuration before the first prompt runs, so the first turn already uses the chosen model. For agents that expose no model configuration surface, the field SHALL remain a silent no-op (the session uses its default model and no model UI is shown), consistent with existing behavior. A model id the agent rejects SHALL surface as a typed error on the session rather than a silent fallback.

#### Scenario: chosen model applies from the first turn

- **WHEN** the operator creates an ACP session with a `model` the agent supports
- **THEN** the session's first turn runs with that model, and the session's `current_model` afterwards reflects it

#### Scenario: agent without a model surface ignores the field

- **WHEN** the operator creates an ACP session with a `model` against an agent that exposes no model configuration
- **THEN** the session spawns with the agent's default model and no error is raised

#### Scenario: rejected model is a typed error

- **WHEN** the operator creates an ACP session with a `model` the agent rejects
- **THEN** the session surfaces a typed rejection and does not silently fall back to the default model

### Requirement: Fetch models from the provider's official base URL

The WebUI provider surface SHALL offer a fetch action that retrieves the model ids the
provider's official base URL currently serves. The action SHALL live inside the
provider editor, next to the Models block heading — the provider row SHALL NOT carry
a fetch button. It SHALL be available for preset-derived and custom providers alike
whose provider has a usable base URL, and SHALL be hidden when there is none. Fetching
SHALL persist nothing by itself: the returned ids replace the editor's in-memory model
list wholesale (deduplicated by id; an existing entry whose id also appears in the
fetched list keeps its manually assigned capability tags), and the replacement reaches
the stored provider only through the editor's normal save. A failed fetch SHALL leave
the editor's model list untouched and report the sanitized reason; a failure SHALL NOT
be presented as an empty successful list.

#### Scenario: fetch lists the official model ids

- **WHEN** the operator opens the provider editor and runs fetch on a provider whose
  base URL serves a model list
- **THEN** the returned ids replace the editor's in-memory model list, and the
  provider's stored data is unchanged until the operator saves the editor

#### Scenario: fetch lists the official model ids into the editor

- **WHEN** the operator opens the provider editor and runs fetch on a provider whose
  base URL serves model ids `m1`, `m2`
- **THEN** the editor's model list is replaced by `m1`, `m2`, and the provider's
  stored data is unchanged until the operator saves the editor

#### Scenario: picking a fetched model edits the list

- **WHEN** a fetch returns model ids and the operator saves the editor
- **THEN** the fetched ids join the provider's stored model list through the ordinary
  editor save, each with the implicit text capability and no invented parameters

#### Scenario: picking tags is preserved for surviving ids

- **WHEN** the editor lists model `m1` tagged `vision` and a fetch returns `m1`, `m2`
- **THEN** after the replacement `m1` still carries its `vision` tag and `m2` starts
  with no capability tags beyond the implicit text capability

#### Scenario: cancelling the editor discards the fetch

- **WHEN** the operator runs fetch and then closes the editor without saving
- **THEN** the provider's stored model list is unchanged

#### Scenario: no base URL means no fetch entry

- **WHEN** a provider has no usable base URL
- **THEN** the editor renders no fetch action for it

#### Scenario: failure is reported honestly

- **WHEN** the upstream fetch fails
- **THEN** the editor's model list keeps its prior content and the surface shows the
  sanitized reason, not an empty list presented as success

### Requirement: Services 分区与 router 状态归属
Services 分区 SHALL 以 watchdog 受管子进程为唯一数据源：调用 `GET /api/admin/services` 获取受管服务表，渲染每个进程的 name / desired / actual status / uptime_secs / 最近错误（由 `/api/admin/events` 提供，无事件则不渲染错误行）。受管服务名固定为 `core` / `webui` / `router` / `im`（IM 在配置未启用时不出现；产品对外名称保留「飞书」由前端做 i18n）；名称不属于受管集合的条目（如 watchdog / updater 等监督器内部角色）SHALL NOT 渲染为服务行。core 为恒启动服务：其行 SHALL 呈纯只读——仅呈现名称与状态，SHALL NOT 渲染 enable / disable / restart 任何动作按钮（enable-core-by-default；core 重启只经 CLI 或升级流程）。

非 core 行的动作按钮 SHALL 按 actual status 驱动互斥显示：`running` 只渲染 ■（disable）；`stopped` / `disabled` 只渲染 ▶（enable）；`starting` / `restarting` SHALL 渲染不可点击的过渡占位（保持动作区列宽不变，不闪现启停钮）；`degraded` / `failed-startup` 渲染 ■ + ⟳。⟳（restart）在非过渡态的非 core 行恒可渲染。任一行动作执行期间（busy）该行全部动作 SHALL 禁用。各行动作区 SHALL 定宽：按钮空缺（core 只读行、过渡占位）以等宽占位填充，使所有行的状态圆点与状态文字纵向对齐到同一水平位置。

无 watchdog adapter 时 SHALL 显式呈现 `adapter_ok: false` 横幅、不暴露 enable/disable/restart 按钮；该形态下 `/api/admin/services` 返回空数组且后端响应携带 `adapter_ok: false`。router 的运行状态（desired / actual / uptime）SHALL 仅由本分区呈现；Models 分区 SHALL NOT 呈现 router 网关总览、listen / debug / auth 或任何 router 运行状态，provider 管理面与 router 运行状态在产品语义上分离。

停止 `router` SHALL 走强制出口流：停止请求被拒（存在活跃 routed 会话，响应携带会话计数）时，前端 SHALL 呈现确认对话框——显示活跃会话计数与后果（流式中断），操作员可取消或选择「强制停止」；强制停止 SHALL 以 `force: true` 重发并被服务端放行。前端 SHALL NOT 预先查询活跃会话数（避免竞态窗口），拒绝驱动弹窗即可；并发竞态（确认期间会话增减）由服务端再次执法兜底。

#### Scenario: Services 渲染受管子进程

- **WHEN** watchdog 拉起 core/webui/router/im 四个进程，Services 分区聚焦
- **THEN** 列表呈现 core / webui / router / im 四行及 desired / actual / uptime；im 在配置未启用时不出现该行；不出现 watchdog / updater 等任何非受管行

#### Scenario: 无 watchdog adapter 退化

- **WHEN** webui 在裸 core 形态下启动（无 SEBAS_CONTROL_SECRET）
- **THEN** Services 分区显示「无 watchdog 控制面」横幅、列表为空、enable/disable/restart 按钮不渲染

#### Scenario: enable 成功

- **WHEN** 操作员对处于 `stopped` 状态的 `router` 点击 ▶
- **THEN** 前端 POST `/api/admin/services/router/enable` 收到 200；列表行刷新（desired 变 on）；若后端返回 503 则内联呈现错误且不刷新

#### Scenario: disable 成功

- **WHEN** 操作员对处于 `running` 状态的辅助服务（如 `router`）点击 ■（前提：confirm 弹窗已确认）
- **THEN** 前端 POST `/api/admin/services/router/disable` 收到 200；列表行刷新；core 行不渲染任何动作按钮

#### Scenario: router 停止被拒呈现强制出口

- **WHEN** 操作员停止 `router` 被拒且响应携带活跃 routed 会话计数
- **THEN** 前端呈现确认对话框（计数与流式中断后果），操作员选择「强制停止」后以 force 重发并成功，列表行刷新为 stopped

#### Scenario: router 停止被拒后取消

- **WHEN** 操作员在强制出口对话框中取消
- **THEN** 不发送任何请求，router 行保持原状

#### Scenario: restart 操作

- **WHEN** 操作员对辅助服务（如 `router`）点击 ⟳ 并确认
- **THEN** 前端 POST `/api/admin/services/router/restart` 收到 200，成功后列表行刷新；core 行不存在 ⟳，restart 对 core 不可达

#### Scenario: core 行完全只读

- **WHEN** core 行渲染（任意状态）
- **THEN** 该行仅呈现名称与状态，无 ▶ / ■ / ⟳ 任何动作按钮

#### Scenario: running 行启停钮互斥

- **WHEN** 某辅助服务 actual status 为 `running`
- **THEN** 该行动作区只渲染 ■，不渲染 ▶

#### Scenario: stopped 行启停钮互斥

- **WHEN** 某辅助服务 actual status 为 `stopped` 或 `disabled`
- **THEN** 该行动作区只渲染 ▶，不渲染 ■

#### Scenario: 过渡态渲染禁用占位

- **WHEN** 某辅助服务 actual status 为 `starting` 或 `restarting`
- **THEN** 该行动作区渲染不可点击的过渡占位，不渲染 ▶ / ■，动作区列宽与其它行一致

#### Scenario: 降级态渲染恢复与下车道

- **WHEN** 某辅助服务 actual status 为 `degraded` 或 `failed-startup`
- **THEN** 该行动作区渲染 ■ 与 ⟳，不渲染 ▶

#### Scenario: 状态列纵向对齐

- **WHEN** Services 列表同时渲染 core（无按钮）、running 辅助服务（■ + ⟳）与过渡态服务（占位）
- **THEN** 三行的状态圆点与状态文字对齐到同一水平位置，动作区以定宽 + 占位填充

#### Scenario: router 状态只在 Services 呈现

- **WHEN** 操作员分别聚焦 Models 与 Services 分区
- **THEN** Models 分区只呈现 provider 列表与管理入口，不渲染 router 的 listen / debug / auth 或可达性；router 的 desired / actual / uptime 只在 Services 分区呈现，且该分区在无 watchdog adapter 时如实说明不可用

### Requirement: Session payload carries the conversation

The session payloads the workbench reads — `GET /api/sessions/{key}` and the
focused session in `GET /api/summary` — SHALL carry the session's conversation as
one ordered entry sequence. Each entry SHALL state its monotonic `position`, its
`kind` (a submission by the operator, or content produced by the agent), its
`element_type`, its content, and its timestamp. Submission entries SHALL be part
of that sequence. The former single `user_prompt` field and the agent-output-only
`body` field SHALL be retired: a client SHALL NOT have to reconstruct the
operator's turns from a separate field, nor infer turn boundaries from timestamps.
A session with no entries SHALL render an honest empty state rather than a failed
payload.

`GET /api/sessions/{key}` SHALL accept an optional `entries_after` query
parameter: when present, the payload's `entries` SHALL contain only entries
whose `position` is greater than the given value, while the rest of the
payload (status, model face, project binding) stays complete; an absent
parameter SHALL keep the full-sequence behavior; a non-numeric value SHALL
be rejected with 400 rather than silently treated as a full fetch. Position
semantics SHALL guarantee incremental-fetch correctness: within a live
session,
positions SHALL be gapless (assigned contiguously from the log length) and
append-only (an appended entry is never rewritten or removed while the
session exists) — so a fetch from `entries_after=N` returns exactly the
entries a client holding position N has not seen.

#### Scenario: both sides of the conversation are in the payload

- **WHEN** the browser requests a session in which the operator submitted messages across several turns
- **THEN** the payload carries one ordered entry sequence containing both the submissions and the agent's output, each entry stating kind and element_type

#### Scenario: retired fields are gone

- **WHEN** a session payload is returned
- **THEN** it carries no single-prompt field and no agent-output-only list, and the conversation is available only as the ordered entry sequence

#### Scenario: empty session is not an error

- **WHEN** a session has no transcript entries yet
- **THEN** the payload returns an empty entry sequence with a success status

#### Scenario: incremental fetch returns only newer entries

- **WHEN** the browser requests the session with `entries_after=5` and the
  transcript holds positions 0..9
- **THEN** `entries` contains exactly positions 6..9 (ordered), and the
  payload's non-entry fields stay complete

#### Scenario: absent parameter keeps full sequence

- **WHEN** the browser requests the session without `entries_after`
- **THEN** `entries` contains the full sequence from position 0 (current
  behavior)

#### Scenario: invalid parameter is a typed rejection

- **WHEN** the browser requests the session with `entries_after=abc`
- **THEN** the response is 400 with an error message, not a silent full
  fetch

#### Scenario: positions are gapless and append-only

- **WHEN** entries are appended to a live session's transcript
- **THEN** each new entry's position equals the transcript length before
  the append (no gaps, no reuse), and previously appended entries keep
  their position and content unchanged

### Requirement: 会话创建携带 mode

`POST /api/sessions` SHALL 接受 `mode` 字段，词汇为 `ask / edit / allow / auto`（与节点链路 `SessionMode` 一致）。创建对话框预填 `ask` 并在 wire 上无条件发送 mode 字段——不存在"缺省 = 省略字段"的空路径；协议层仍容忍缺省/null（按 `ask` 以外的执行体默认处理），但 WebUI 自身的创建路径恒发送显式值。未知词汇 SHALL 返回 400 指出非法值，SHALL NOT 静默降级为默认。`mode` 与 `agent`/`model` 一样在创建时记入会话映射；0-turn 占位会话 SHALL 记住请求的 mode，并在首条消息触发 spawn 时应用。远端节点上的 0-turn 占位仍按既有规则如实拒绝。

#### Scenario: 创建会话带 mode=allow

- **WHEN** 客户端 `POST /api/sessions` 携带 `{"agent":"claude","prompt":"...","mode":"allow"}`
- **THEN** 会话以 mode=allow 建立，后续快照可见请求的 mode

#### Scenario: 未知 mode 如实拒绝

- **WHEN** 客户端 `POST /api/sessions` 携带 `"mode":"plan"`
- **THEN** 返回 400 并指出 mode 非法，不创建会话

#### Scenario: 占位会话记住 mode

- **WHEN** 不带 prompt 创建会话且携带 `mode=edit`
- **THEN** 建立 0-turn 占位，不 spawn 子进程；首条消息到达时以 edit 的语义 spawn

### Requirement: 会话中途切换 mode

WebUI SHALL 暴露 `POST /api/sessions/{key}/mode`（请求体 `{"mode": "<ask|edit|allow|auto>"}`，同创建词汇与校验）。切换 SHALL 沿会话放置路径送达执行体：本机 claude 会话经 driver 运行时权限模式切换，远端节点会话经节点链路 SetMode。执行体接受与否 SHALL 经事件流反馈：接受后会话快照的 mode 更新；拒绝或执行体做不到时 SHALL 产生非致命错误事件（UI 显示错误、mode 保持原值），SHALL NOT 终止会话。

#### Scenario: 本机会话切换成功

- **WHEN** 对运行中的本机 claude 会话 POST `/api/sessions/{key}/mode` with `{"mode":"allow"}`
- **THEN** driver 收到运行时权限模式切换并回 `ModeChanged`；快照 mode 更新为 allow

#### Scenario: 执行体拒绝切换不致命

- **WHEN** 切换请求被执行体拒绝（如 agent 不支持该模式）
- **THEN** 会话收到一条非致命错误事件（UI 可见），快照 mode 保持原值，会话继续可用

### Requirement: 会话 mode 在 dashboard 可见可切

会话 dashboard SHALL 展示当前会话的 mode（含远端会话已有的 desired/effective 呈现），切换入口 SHALL 位于输入框底沿左端，与模型芯片、提交按钮同一工具条，提交后走中途切换端点。会话头部 SHALL NOT 渲染 mode 切换控件。mode 显示对远端节点会话沿用 effective/desired 差异化呈现（effective 缺失时只显 desired）。

#### Scenario: composer 创建表单的 mode 选择

- **WHEN** 操作者在创建对话框展开表单
- **THEN** 表单提供 mode 下拉，缺省项为「agent 默认」（不发送 mode 字段）

#### Scenario: 会话头部切换 mode

- **WHEN** 操作者在输入框底沿的 mode 下拉选择另一个 mode
- **THEN** 前端提交 `POST /api/sessions/{key}/mode`；成功后下拉显示的 mode 更新，失败显示非致命错误且保持原显示

### Requirement: SessionBackend seam 承载 mode

`SessionBackend` 的 spawn / placeholder / 切换方法 SHALL 携带 mode 维度（与 agent/model/node 同构）：进程内后端把 mode 交给本机执行体映射；分离部署经核心通道 Spawn 帧携带 mode；远端放置把 mode 交给节点投影。 seam 的默认实现对 mode 的降级 SHALL 如实（不支持 mode 的后端不假装生效）。

#### Scenario: 分离部署 wire 携带 mode

- **WHEN** webui 与 core 分离部署、创建会话带 mode
- **THEN** 核心通道 Spawn 帧携带该 mode，core 侧按同一映射语义放置

#### Scenario: 不支持 mode 的执行体如实回报

- **WHEN** mode 发给无法生效它的执行体（如 native 内核、未声明 mode 能力的通用 ACP agent）
- **THEN** 创建成功但该执行体不声称 mode 生效；能回报 effective 的位置如实回报空

### Requirement: 环境变量只读展示（/api/env）

WebUI SHALL 提供只读端点 `GET /api/env`：读取 **webui 进程自身**的环境变量，返回**策划过**的变量清单（每项含名字、人读解释、按分类的值展示），经既有鉴权。策划清单之外的环境变量（内部管道、测试专用）SHALL NOT 出现在响应中。清单分类与展示语义：

- **非敏感**（路径与开关，如 `SEBAS_STATE_DB`、`SEBAS_STATE_FILE`、`SEBAS_ROUTER_CONFIG`、`SEBAS_ROUTER_PROVIDER_OVERLAY`、`SEBAS_LOG_LEVEL`、`SEBAS_HANG_TIMEOUT_SECS`、`SEBAS_FEISHU_APP_ID`）：已设置显示实际值；未设置标注「未设置（用默认）」且解释里写明默认值。
- **敏感**（凭据类，如 `SEBAS_WEBUI_PASSWORD`、`SEBAS_CONTROL_SECRET`、`SEBAS_FEISHU_APP_SECRET`）：SHALL 只显示「已设置 / 未设置」，**值本身 SHALL NOT 出现在响应里**（遮蔽在服务端完成）。

前端 `Env Vars` 分区 SHALL 消费该端点渲染只读表；端点失败（core 无关，纯 webui 面）时分区 SHALL 如实呈现错误状态而非空白或编造数据。

#### Scenario: 非敏感变量显示实际值或默认标注

- **WHEN** `GET /api/env` 返回且 `SEBAS_STATE_DB` 已设置
- **THEN** 该项显示实际路径值；`SEBAS_STATE_FILE` 未设置时该项显示「未设置（用默认）」且解释含默认路径

#### Scenario: 敏感值永不出现在响应

- **WHEN** `SEBAS_FEISHU_APP_SECRET` 已设置且操作员请求 `GET /api/env`
- **THEN** 响应中该项只标「已设置」，任何字段都不含其值明文

#### Scenario: 内部与测试变量不列

- **WHEN** webui 进程环境中存在内部管道或测试专用变量（如 `SEBAS_IPC`、`SEBAS_TEST_SPAWN_SESSION`）
- **THEN** 它们不出现在 `/api/env` 响应与 Env Vars 分区表中

#### Scenario: 端点经既有鉴权

- **WHEN** webui 鉴权开启且未登录的客户端请求 `GET /api/env`
- **THEN** 请求被既有鉴权拒绝，与其他 `/api` 端点一致

#### Scenario: 端点失败如实呈现

- **WHEN** `GET /api/env` 请求失败（非 200）
- **THEN** Env Vars 分区呈现明确的错误提示，不渲染编造或空表假象

### Requirement: 鉴权开关（auth）与首启用户引导

WebUI SHALL 提供 `[service.webui] auth` 配置开关，默认 `true`。开关为 `true` 时，鉴权门 SHALL 恒在：`/api/*`、`/router/api/*`、`/ws` 需要有效 会话；用户库（auth.db）零用户时 SHALL 不自动生成任何凭据，改为进入 首启引导流程（见 `webui-user-management` 能力：设置页或环境变量建立 root），期间 `GET /api/auth/me` SHALL 报告 `needs_setup: true`。开关为 `false` 时，无论用户库是否存在用户，SHALL 对所有路由（含静态资源） 完全放行，不要求登录且不触发引导；`GET /api/auth/me` SHALL 报告 `enabled: false`（前端据此不渲染登录页）。`sebas auth`（`add`/`passwd`/`list`）在开关关闭时仍可管理用户（为重新启用做准备），但不产生任何强制登录效果。

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

### Requirement: 多用户登录形态

`POST /api/auth/login` SHALL 只接受 `{"username", "password"}` 一种
形态（缺失字段返回 400）。成功即建立
绑定该用户的会话 cookie；凭据失败统一 401，不区分「用户不存在」与
「密码错误」，限速策略不变（按来源 IP）。登录页 SHALL 呈现用户名 +
密码两个字段。

#### Scenario: 用户名密码登录

- **WHEN** `{"username", "password"}` 提交到 `/api/auth/login` 且凭据
  正确
- **THEN** 登录成功并建立绑定该用户的会话，响应携带该用户名

#### Scenario: 失败不泄漏用户存在性

- **WHEN** 提交不存在的用户名或错误密码
- **THEN** 响应统一为 401，文案不区分两种失败，响应时序无可区分的
  快慢差

#### Scenario: 登录页两字段

- **WHEN** 前端渲染登录门
- **THEN** 登录表单有用户名与密码两个输入框，401 就地提示「凭据错误」

### Requirement: 分级通知层

WebUI SHALL 提供唯一的视口级顶部居中通知层，按四级呈现全局通知：info（蓝色瞬
时 toast，数秒自动消失）、warn（琥珀色：瞬时来源用自动消失 toast，持续状态用
驻留横幅）、error（红色瞬时 toast，默认 8 秒自动消失；调用点显式指定驻留
（duration=0）时仍为驻留条、须操作者手动关闭）、fatal（驻留横幅 + 锁定，语义
见「全局核心可达性横幅」）。判级 SHALL 由前端按影响面裁定——应用瘫痪 =
fatal、单个视图 / 能力不可用 = error、单个操作失败且可立即重试 = warn、无损
状态提示 = info；HTTP 状态码只是信号、不直接定级。API 客户端 SHALL 对未豁免
的请求失败自动按级弹出（操作失败类 → warn），而有内联错误呈现的表单调用点与
带重试的列表加载 SHALL 豁免统一拦截、维持内联呈现；401 SHALL 走既有登录跳
转、SHALL NOT 进入通知层。通知层 SHALL 满足：瞬时 toast 栈上限三条、超出挤
掉最旧瞬时条（显式驻留条不参与挤占；error 默认条按瞬时条参与挤占）；同一文
案在去重窗口内 SHALL NOT 重复弹出；info / warn / error toast 可提前手动关
闭；fatal 横幅不可手动关闭。既有「与服务器的连接已断开」横幅 SHALL 收编为本
层的持续 warn 驻留横幅，旧实现 SHALL 移除。通知层与 settings 弹窗等浮层叠放
时 SHALL 保持在上；窄屏 SHALL 退化为全宽贴顶。

#### Scenario: 四级形态可辨识

- **WHEN** info / warn / error / fatal 各级通知呈现
- **THEN** 四级在配色上可区分（info 蓝、warn 琥珀、error 红、fatal 红 + 锁定遮罩），且颜色不是唯一的信息通道（附图标与文案）

#### Scenario: API 操作失败自动弹 warn

- **WHEN** 一个未豁免的 API 调用因网络失败或服务端错误而失败（如保存请求超时）
- **THEN** 通知层弹出 warn 级失败提示并自动消失，该调用点无需自行处理全局呈现

#### Scenario: 内联错误点不双弹

- **WHEN** composer 提交、settings 保存或列表初始加载等有内联错误呈现的调用点失败
- **THEN** 错误维持内联呈现，通知层不重复弹出同一失败

#### Scenario: error 默认自动消失且参与挤占

- **WHEN** 视图 / 能力级故障由视图显式上报为 error 级通知，且未显式指定驻留
- **THEN** 该通知默认 8 秒自动消失，并作为瞬时条参与三条栈上限的挤占（持续型
  故障由 fatal 横幅槽位承载，不依赖 error toast 驻留）

#### Scenario: 驻留 error 须手动关闭

- **WHEN** 调用点显式以 duration=0 上报 error 级通知（要求驻留）
- **THEN** 该通知不自动消失、不参与瞬时栈挤占，操作者手动关闭后即移除

#### Scenario: 栈上限与去重

- **WHEN** 瞬时通知超过三条，或同一文案在去重窗口内重复触发
- **THEN** 超出时挤掉最旧的瞬时条，重复文案不产生第二条

#### Scenario: WS 断线收编为持续 warn 驻留横幅

- **WHEN** `/ws` 连接断开
- **THEN** 持续 warn 驻留横幅出现在通知层（旧 app-shell 横幅不再渲染），既有指数退避重连继续；重连成功后横幅消失并触发既有 `sebas:refetch` 刷新

#### Scenario: 鉴权拒绝型断开不亮断连横幅

- **WHEN** 登录态下 `/ws` 因鉴权被拒而断开（而非网络断开），且重连退避在运行
- **THEN** 不弹出「服务器断开」warn 横幅（避免误导为服务故障）；页面就绪即重连并清空退避，连接恢复后一切照常
