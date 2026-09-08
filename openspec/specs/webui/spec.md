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
optional `prompt` field), `GET
/api/sessions/{key}`, `POST /api/sessions/{key}/message`, `POST
/api/sessions/{key}/close`, `POST /api/sessions/{key}/switch`, `GET
/api/summary`, `POST /api/permissions/{request_id}/answer`, `GET /api/settings`,
`GET /api/router`, `GET /api/about`, `GET /api/agent-defaults` and `PUT
/api/agent-defaults` (the default provider and model new sessions start with,
read and set through the router admin defaults surface), the project APIs
`GET /api/projects` and `POST /api/projects` (register), `POST
/api/projects/reorder`, `POST /api/projects/{path}/remove`, `GET
/api/projects/{path}/branch`, `GET /api/fs/browse-dirs` (lazy directory listing
for the folder picker, scoped to the server's work directory — the configured
work dir of the default agent kind, falling back to the WebUI process working
directory; an explicit `root` query parameter overrides the default), `POST
/api/sessions/{key}/archive` (archive a session), `POST
/api/sessions/{key}/restore` (restore an archived session), `GET /api/archive`
(list archived sessions with expiry info), and `GET /ws`
(WebSocket session stream). Project and session mutations are POST-only and
carry the same posture as the existing session APIs. The router mutation
cluster — POST/PUT/DELETE under `/router/api/*` (provider and model-alias
CRUD, provider probe, defaults, reload) — is functional only when a control
secret is configured; without it the mutations return 503. Router data is
fetched live from the router admin API at request time (proxied server-side by
the WebUI backend with the control secret), not from a startup snapshot. The
JSON admin API `/api/admin/*` (status, events, services, login, logout,
update, update/dry-run, update/dev, rollback, restart) is always mounted:
without a control-plane adapter its reads report `adapter_ok: false` and its
mutations return 503 (honest degradation). `GET /health` returns the literal
`ok`. All browser assets the UI needs to render — styles, fonts, Web Awesome,
markdown rendering, and syntax highlighting — are self-hosted under
`/assets/*`; the UI SHALL NOT depend on an external CDN at render time.
Navigation SHALL only link to routes this surface serves.

`GET /api/fs/browse-dirs` SHALL honour a path round-trip contract: the `path`
echoed in a listing response SHALL be accepted verbatim as the `path` of a
subsequent request for that same directory, and request paths that mix `/` and
`\` separators SHALL resolve to the same directory. The echoed path SHALL NOT
carry a Windows verbatim (`\\?\`) prefix.

#### Scenario: dashboard route

- **WHEN** a browser requests `/`
- **THEN** the SPA workbench renders, listing registered projects in the
  project rail, the Inbox grouping for sessions with no project, the History
  (archive) group, and the selected project's sessions

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

- **WHEN** a provider is renamed through the router admin API and the
  browser then requests `GET /api/router`
- **THEN** the response lists the new provider name without a WebUI restart

#### Scenario: router mutations unavailable without secret

- **WHEN** the WebUI runs without a control secret and a mutation is posted
  to `/router/api/providers`
- **THEN** the response is 503

#### Scenario: agent defaults read and set

- **WHEN** the operator sets a default provider and model and the browser
  then requests `GET /api/agent-defaults`
- **THEN** the response reflects the stored default without a restart, and
  with no control secret `PUT /api/agent-defaults` returns 503

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
- **THEN** the session is archived, moved from the active session list, and the response confirms the action

#### Scenario: archive list endpoint

- **WHEN** `GET /api/archive` is called
- **THEN** the response lists all archived sessions with their original project, archive timestamp, and expiry

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

### Requirement: 降级与错误表现

The WebUI frontend SHALL surface a visible global indicator when its live
connection to the server (`/ws`) is lost, and SHALL clear that indicator and
refresh visible view data automatically when the connection is restored
(the existing refetch hook). Data requests that fail at the network level
(server process down, DNS/connection failure) SHALL be distinguishable from
server-side business errors (4xx/5xx with a backend error body) so views can
react appropriately. List-style views (dashboard, project rail, sessions)
SHALL render an inline failure state with a retry affordance instead of a
blank panel when their initial data load fails. The workbench composer SHALL
treat a failing `/api/summary` poll as core-unreachable for its submit gate,
consistent with a reported `reachability.ok = false`.

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

- **WHEN** the composer's periodic `/api/summary` request itself fails
- **THEN** the submit gate treats the core as unreachable, matching the
  behavior for a reported `reachability.ok = false`

### Requirement: Local-only binding

The standalone WebUI SHALL default to a loopback bind (`127.0.0.1:9797`).
The legacy `core --webui` path binds hard-coded `127.0.0.1`. A non-loopback
`watchdog.webui.host` SHALL be refused with a configuration error unless the
authentication switch is enabled and login credentials exist (see
「鉴权开关（auth）」and「非 loopback bind 与开关联动」below).

#### Scenario: non-loopback refused without auth

- **WHEN** the config sets `watchdog.webui.host = "0.0.0.0"` while login
  credentials are absent, or while `auth = false`
- **THEN** `sebas webui` exits with a configuration error rather than
  binding

### Requirement: 鉴权开关（auth）

WebUI SHALL 提供 `[watchdog.webui] auth` 配置开关，默认 `true`。
开关为 `true` 时，登录鉴权行为不变：凭据文件存在即启用鉴权门，`/api/*`、
`/router/api/*`、`/ws` 需要有效会话。开关为 `false` 时，即使凭据文件存在，
SHALL 对所有路由（含静态资源）完全放行，不要求登录；`GET /api/auth/me`
SHALL 报告 `enabled: false`（前端据此不渲染登录页）。`sebas webui-passwd`
在开关关闭时仍可管理凭据（为重新启用做准备），但不产生任何强制登录效果。

#### Scenario: 默认打开

- **WHEN** 配置未写 `auth` 且凭据文件存在
- **THEN** 未带会话的 `/api/summary` 请求返回 401，行为与无开关时一致

#### Scenario: 测试环境关闭

- **WHEN** 配置设置 `watchdog.webui.auth = false` 且凭据文件存在
- **THEN** 未带任何会话的 `/api/summary` 请求返回 200，全部路由免登录
- **AND** `GET /api/auth/me` 返回 `{"enabled": false, "authenticated": false}`

#### Scenario: 关闭后重新打开立即生效

- **WHEN** 开关从 `false` 改回 `true` 并重启 webui
- **THEN** 已存在的凭据立即恢复强制登录，无需重建凭据文件

### Requirement: 非 loopback bind 与开关联动

当 `watchdog.webui.host` 非 loopback 时，webui SHALL 仅在
`auth = true`（或缺省）且登录凭据存在时才允许绑定启动；否则 SHALL 以配置
错误拒绝启动。开关关闭时无论凭据是否存在，SHALL 拒绝非 loopback bind
（防止误关开关叠加公网暴露）。

#### Scenario: 开关关闭拒绝公网 bind

- **WHEN** 配置同时设置 `auth = false` 与 `host = "0.0.0.0"`
- **THEN** `sebas webui` 以配置错误退出，不绑定端口

#### Scenario: 开关打开且凭据存在允许公网 bind

- **WHEN** 配置设置 `auth = true`（或缺省）、`host = "0.0.0.0"`，
  且凭据文件存在
- **THEN** `sebas webui` 正常绑定并在日志中提示已启用登录鉴权

### Requirement: Optional admin authentication

Admin authentication SHALL be enabled only when `SEBAS_WEBUI_PASSWORD` is
set: unauthenticated admin routes redirect to `/admin/login`; a successful
login sets an HttpOnly, SameSite=Lax session cookie scoped to `/admin` with
a 24 h inactivity TTL. Login attempts are rate-limited to 5 per 30 s. When
no password is configured, admin reads are served without authentication.
The non-admin pages and session APIs have no authentication.

#### Scenario: password-gated admin

- **WHEN** `SEBAS_WEBUI_PASSWORD` is set and an unauthenticated request
  hits `/admin/status`
- **THEN** the response redirects to `/admin/login`

#### Scenario: login lockout

- **WHEN** 6 login attempts with a wrong password arrive within 30 s
- **THEN** the attempts are rejected by the rate limiter

### Requirement: Mutation posture

Admin mutations SHALL be POST-only (non-POST gets 405) and guarded by an
origin check: empty origin or loopback origin (`127.0.0.1`, `localhost`,
`::1`) passes; a non-loopback origin requires a valid CSRF token when a
password is set, else 403. Router mutation routes under `/router/api/*`
follow the same posture: POST-only with the same origin check, and the
WebUI forwards them to the router admin API server-side with the control
secret — the secret never reaches the browser. In the shipped UI, browser
buttons post with a loopback origin, which is the operative authentication
path for mutations.

#### Scenario: post-only

- **WHEN** a GET hits `/admin/restart`
- **THEN** the response is 405

#### Scenario: foreign origin rejected

- **WHEN** a mutation POST carries `Origin: https://evil.example`
- **THEN** the response is 403

#### Scenario: router mutation is post-only and origin-checked

- **WHEN** a GET hits `/router/api/providers` or a router mutation POST
  carries a non-loopback origin without a valid CSRF token
- **THEN** the response is 405 (GET) or 403 (foreign origin)

### Requirement: Session dashboard and focus semantics

The cross-project session list SHALL render one row per known session (encoded
key, chat and thread ids, session id, status, phase, relative last-active),
active-first, and SHALL be reachable from the workbench rather than from
primary navigation. The session list SHALL exclude archived sessions — those
are served by `GET /api/archive`. Visiting a session's detail page or posting
`/switch` SHALL
set the webui-side focused session — a display pointer only that never changes
message routing — and `switch` returns the redirect target or 404 for an
unknown key. Switching the displayed project SHALL NOT alter the focused
session pointer.

#### Scenario: focus is cosmetic

- **WHEN** the user focuses session B in the WebUI while session A is
  active
- **THEN** subsequent Feishu messages still route per the router's own
  session mapping, unchanged

#### Scenario: switch unknown key

- **WHEN** `/api/sessions/{key}/switch` posts a key not in the map
- **THEN** the response is 404

#### Scenario: project switch leaves focus alone

- **WHEN** the operator switches the displayed project
- **THEN** the focused session pointer is unchanged

#### Scenario: archive hides session from list

- **WHEN** a session is archived
- **THEN** it is no longer returned by `GET /api/sessions` and appears only in the `GET /api/archive` response

### Requirement: Web session close

`POST /api/sessions/{key}/close` SHALL kill the ACP child when the mapping
is active, drop the mapping and card state, clear the chat-level permission
allowlist and reply target, and clear the focused-session pointer if it
pointed at the closed session. Dormant mappings drop without a kill. Unknown
keys return 404. Confirmation is client-side only (detail-page banner);
dashboard close buttons act immediately.

#### Scenario: close active session

- **WHEN** the user closes an active session from the detail page
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
- **THEN** allow / deny 各一条旅程全绿；单进程形态的同名用例保持全绿（两种拓扑不互相代替）。本 scenario 阻塞于 `wire-webui-sebas-agent-e2e` 任务 1.3（审批通道接线）+ harden 5.4 harness 落地，见 tasks B 批

#### Scenario: approval_answer rejects unknown request_id

- **WHEN** webui POST `/api/permissions/{rid}/answer` with `rid` 不存在
- **THEN** 后端回 4xx typed rejection（`UnknownRequestId` 或同类）；前端不渲染成功状态

#### Scenario: set_session_model happy-path via webui

- **WHEN** 沙箱 fake-claude 启动带 `configOptions.model` 多 model、用户 PUT `/api/sessions/{key}/model` with `{model_id: "<另一 model>"}`
- **THEN** 后端走 channel SetSessionModel；acp 子进程回 `ModelChanged` 事件；webui snapshot 同步 `current_model`；前端 UI 显示新 model selected

#### Scenario: set_session_model rejects unknown model

- **WHEN** 用户 PUT `/api/sessions/{key}/model` with `{model_id: "<不存在的 model>"}`
- **THEN** 后端走 channel SetSessionModel；acp 子进程回 `Error`（typed rejection）；webui 端呈现内联错误；session state 不变

#### Scenario: cross_uid rejection covers live process

- **WHEN** live fork + setuid 到不同账户进程尝试连接 core socket 并发 Snapshot 请求
- **THEN** 连接被拒；服务端日志写 peer-uid mismatch；不进入 Snapshot 处理路径

#### Scenario: backend push renders faithfully

- **WHEN** backend 推送 spawn-failure 事件
- **THEN** 前端立即渲染该事件为 transcript 内显错误，不等待 Removed 事件；会话状态 SHALL 在 backend 推送 Removed 之前已经标记为 spawn-failed

### Requirement: Admin actions via control plane
Admin mutations SHALL proxy to the watchdog over the control RPC using the `SEBAS_CONTROL_SECRET`, attributed to a local CLI actor — which executes directly without a confirmation round-trip. **补充**：Actions 在原有 update (release)、update dry-run、update dev、rollback、restart core 之外，新增「Services 分区内的 enable/disable/restart」三类——enable/disable 经 `POST /api/admin/services/{name}/enable|disable` 走 `ServiceSet` RPC；restart per-process 经既有 watchdog 监督循环（每个受管服务都有 restart 入口）。**补充**：裸 core 形态下所有 admin mutations SHALL 返回 503 "control plane not connected"，Services 分区 SHALL 据此呈现退化。

#### Scenario: restart via admin

- **WHEN** the admin clicks restart on `/admin/update`-style pages with the control secret present
- **THEN** the watchdog receives `RestartCore` and restarts the core; the WebUI itself stays up

#### Scenario: no control plane

- **WHEN** the standalone WebUI has no `SEBAS_CONTROL_SECRET`
- **THEN** admin mutation buttons return 503; Services 分区显示「无 watchdog 控制面」横幅

### Requirement: Watchdog lifecycle ownership

The WebUI SHALL be spawned by the watchdog as a separate process (with the
control secret) by default — `[watchdog.webui] enabled` defaults to `true`
unless explicitly set to `false` — and SHALL survive core restarts. The
WebUI SHALL bind to `127.0.0.1:9797` by default; port conflict with a
legacy `core --webui` (or any other process) is resolved by kernel-level
bind atomicity — the first to bind wins, the second bind fails with a
distinct exit code.

#### Scenario: single owner

- **WHEN** the watchdog-spawned WebUI is running and a legacy
  `core --webui` is attempted
- **THEN** the second start is refused by the ownership guard (port
  already bound)

#### Scenario: default enablement

- **WHEN** the watchdog starts with a configuration that contains no
  `[watchdog.webui]` section
- **THEN** the watchdog spawns and supervises the WebUI child process

#### Scenario: explicit disable

- **WHEN** the configuration sets `[watchdog.webui] enabled = false`
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

The archive registry SHALL persist to its own file, separate from the project registry and the router state file. Each entry SHALL record the session key, the original project path, the session label, the archive timestamp, and the retention deadline.

#### Scenario: archive survives restart

- **WHEN** the WebUI process is restarted after sessions were archived
- **THEN** the same archived sessions are listed

#### Scenario: archive expiry clean on startup

- **WHEN** the WebUI starts and an archived session has passed its retention deadline
- **THEN** that entry is removed from the archive file and the session is no longer listed

### Requirement: Configurable archive retention

The WebUI config SHALL support an `archive_retention_days` field under the `[webui]` section, with a default of 30 days. The expiry check SHALL run at WebUI startup and on every `GET /api/archive` or `GET /api/sessions` request.

#### Scenario: default retention

- **WHEN** no `archive_retention_days` is set in the config
- **THEN** the default retention of 30 days applies

#### Scenario: custom retention

- **WHEN** `[webui] archive_retention_days = 60` is set
- **THEN** archived sessions are retained for 60 days

### Requirement: Provider management page

The WebUI settings SHALL provide a provider management page backed by the
router admin API (via the WebUI's `/router/api/providers*` proxy). It
SHALL support: listing providers with name, preset-or-custom mark, base
URL slots, and key-configured state; creating a provider either from a
preset or as custom; editing an existing provider; deleting a provider;
and probing models. Preset-derived providers SHALL present base URLs and
models as read-only code-owned values (labeled as following the code
table) with only the API key, default model, and protocol editable;
custom providers SHALL present all three base URL slots and `api_key_env`
as editable. Secret inputs SHALL never be pre-filled, and an empty secret
submit SHALL preserve the stored key. Mutations SHALL go through the
existing POST-only, origin-checked proxy and surface its errors (409
duplicate, 400 validation, 503 unavailable) in the page.

#### Scenario: preset-derived provider shows read-only URLs

- **WHEN** the user opens provider `glm` (preset-derived) for editing
- **THEN** the base URL fields render the preset's code-owned values as
  read-only with a follow-the-code indication, and only the API key,
  default model, and protocol inputs are editable

#### Scenario: create from preset

- **WHEN** the user creates provider `my-glm` from preset `glm` entering
  only an API key
- **THEN** the create request carries the preset selection and key but no
  base URL or models fields, and the provider appears in the list

#### Scenario: custom provider full editing

- **WHEN** the user creates a custom provider filling
  `base_url_openai_chat` only
- **THEN** the create succeeds and the edit form exposes all three slots
  for later adjustment

#### Scenario: probe from the page

- **WHEN** the user clicks probe on a provider with an OpenAI-family slot
- **THEN** the page shows the returned model list without leaving the page

#### Scenario: delete reflects immediately

- **WHEN** the user deletes provider `alpha` from the page
- **THEN** the provider disappears from the list without a page reload

### Requirement: Provider status parity across deployment forms

The WebUI's provider-derived surfaces (`GET /api/settings` 的 gateway 段、
`GET /api/gateway`、`GET /api/about` 的 provider 计数，及 composer 的
provider 标签）SHALL 在 `run --webui` 与 `sebas webui` 两种部署形态下，对同一
配置呈现一致且真实的 provider 状态。detached 形态 SHALL NOT 以空占位
（`GatewayInfo` 缺省值）作为最终数据源：provider 列表 SHALL 来自 webui 可达的
provider 真源（状态库），gateway 静态事实（listen、debug、has_auth）SHALL
来自配置解析。当 provider 真源不可用时，响应 SHALL 如实标注不可用，而不是
报告"未配置 provider"。

#### Scenario: detached 与 in-process 的 provider 标签一致

- **WHEN** 同一份含已注册 provider 的配置分别以 `run --webui` 与
  `sebas webui`（core 经通道在跑）启动，浏览器打开工作台 composer
- **THEN** 两者的 provider 标签显示相同的 provider 名，而非 detached 侧显示
  "no provider configured"

#### Scenario: detached 反映运行期 provider 变更

- **WHEN** 操作员经 gateway admin API 新增或改名 provider 后刷新 detached
  WebUI 的 settings
- **THEN** 响应中的 provider 集合反映该变更，无需重启 webui 进程

#### Scenario: provider 真源不可用时如实上报

- **WHEN** detached webui 无法从状态库读取 provider 数据
- **THEN** `/api/settings` 的 gateway 段携带可辨识的"不可用"指示，而不是把空
  集合冒充"未配置"

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
设置弹窗 SHALL 暴露分区导航，分区 SHALL 按下列顺序排列：`Settings`（弹窗壳/总览）→ `Services`（受管子进程）→ `Models`（provider 路由）→ `Appearance` → `Env` → `About`。打开弹窗时缺省聚焦 `Settings` 分区；用户上次停留分区 SHALL 在新会话首次打开时被记住（localStorage），之后打开仍按记忆回到上次分区。`Settings` 分区 SHALL 展示工作区根目录、当前 default agent kind、当前 default provider/model 三个只读总览项（数据分别来自既有 `/api/summary`、`/api/agent-defaults`）；以及高危动作入口「全部进程重启（watchdog 形态下可见）/ 重置 Settings」，均 SHALL 经二次确认后生效。

#### Scenario: 缺省聚焦 Settings 分区

- **WHEN** 操作员从侧栏底部打开设置弹窗，且无历史记忆
- **THEN** 弹窗打开后左导航高亮 `Settings`，主区渲染 `Settings` 分区内容

#### Scenario: 历史记忆恢复上次分区

- **WHEN** 操作员上次停留在 `Services` 后关闭弹窗，再打开
- **THEN** 弹窗缺省聚焦 `Services` 分区

#### Scenario: Settings 分区总览

- **WHEN** 聚焦 `Settings` 分区
- **THEN** 主区呈现工作区根目录（路径 + 复制按钮）、default agent kind（只读）、default provider/model（只读 + 跳转 Models 分区的链接）

#### Scenario: 高危动作二次确认

- **WHEN** 操作员点击「全部进程重启」或「重置 Settings」
- **THEN** 弹出确认对话框，描述动作影响与不可撤销性；确认后调用相应后端接口并内联呈现 success/error；无 watchdog adapter 时按钮全灰且 tooltip 标注「无 watchdog 控制面」

### Requirement: Services 分区数据源
Services 分区 SHALL 以 watchdog 受管子进程为唯一数据源：调用 `GET /api/admin/services` 获取受管服务表，渲染每个进程的 name / desired / actual status / uptime_secs / 最近错误（由 `/api/admin/events` 提供，无事件则不渲染错误行）。受管服务名固定为 `core` / `webui` / `router` / `im`（IM 在配置未启用时不出现；产品对外名称保留「飞书」由前端做 i18n）。无 watchdog adapter 时 SHALL 显式呈现 `adapter_ok: false` 横幅、不暴露 enable/disable/restart 按钮；该形态下 `/api/admin/services` 返回空数组且后端响应携带 `adapter_ok: false`。Models 分区顶部 SHALL 呈现 provider 路由网关总览（来自既有 `/api/router`：listen / debug / auth 三行），与 Services 分区在产品语义上彻底解耦。

#### Scenario: Services 渲染受管子进程

- **WHEN** watchdog 拉起 core/webui/router/im 四个进程，Services 分区聚焦
- **THEN** 列表呈现 core / webui / router / im 四行及 desired / actual / uptime；im 在配置未启用时不出现该行

#### Scenario: 无 watchdog adapter 退化

- **WHEN** webui 在裸 core 形态下启动（无 SEBAS_CONTROL_SECRET）
- **THEN** Services 分区显示「无 watchdog 控制面」横幅、列表为空、enable/disable/restart 按钮不渲染

#### Scenario: enable 成功

- **WHEN** 操作员对 `router` 点击 enable
- **THEN** 前端 POST `/api/admin/services/router/enable` 收到 200；列表行刷新（desired 变 on）；若后端返回 503 则内联呈现错误且不刷新

#### Scenario: disable 成功

- **WHEN** 操作员对 `core` 点击 disable（前提：confirm 弹窗已确认）
- **THEN** 前端 POST `/api/admin/services/core/disable` 收到 200；列表行刷新

#### Scenario: restart 操作

- **WHEN** 操作员对 `core` 点击 restart
- **THEN** 前端走 `Admin actions via control plane` 既有 restart-core 路径；成功后列表行 uptime 重置

#### Scenario: Models 分区承载 router 总览

- **WHEN** 操作员聚焦 Models 分区
- **THEN** 主区顶部呈现 provider 路由网关卡片（listen / debug / auth 三行），下方为 provider 列表；不再包含 Services 标签

### Requirement: 全局核心可达性横幅

app-shell SHALL 提供全局"核心不可达"横幅：当 `/api/summary` 的 `reachability.ok` 为 false 时展示，内容含 cause（如 `socket absent`），呈现层级与现有"与服务器的连接已断开"横幅一致（全局、role=alert）；可达性恢复后横幅 SHALL 消失。横幅 SHALL 不阻塞页面其余部分的浏览（与"webui 在 core 不可达时继续服务"的既有语义一致）。

#### Scenario: core 不可达时横幅出现

- **WHEN** core 进程停止或通道不可达，浏览器停留在任意页面
- **THEN** 全局横幅出现且文本包含 `reachability` 上报的 cause

#### Scenario: core 恢复后横幅消失

- **WHEN** core 恢复服务且通道重新握手成功
- **THEN** 无需刷新页面，横幅在下一次可达性轮询后消失

### Requirement: 项目注册降级如实提示

`POST /api/projects` 在状态库（核心通道）不可用而落到本地文件注册表时，响应 SHALL 携带降级标记（含 cause 语义），前端 SHALL 就地提示"核心不可达，已写入本地注册表"一类的如实文案；状态库可用时响应不含该标记，前端无降级提示。两者皆失败时保持既有 503 行为。

#### Scenario: 核心不可达时加项目获得降级提示

- **WHEN** 核心通道不可达时通过 UI 注册一个合法项目目录
- **THEN** 注册成功（201）且响应携带降级标记，项目栏出现该项目并伴随降级提示，用户不再直到新建会话才得知核心不可达

#### Scenario: 核心正常时无降级提示

- **WHEN** 核心通道正常时注册项目
- **THEN** 响应无降级标记，UI 仅呈现常规成功路径
