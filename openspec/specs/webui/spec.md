# webui Specification

## Purpose
Defines the local web dashboard: its HTTP route surface, the local-only
security baseline (loopback binding, optional admin password, mutation
guards), the session dashboard and its focus/close semantics, the detached
behavior of the watchdog-spawned instance, and the admin actions proxied to
the watchdog control plane.

## Requirements

### Requirement: HTTP route surface

The WebUI SHALL serve `GET /` as the SPA shell for the project workbench and `GET /assets/*` for its built styles, scripts, and fonts. Any other browser-facing GET (for example `/sessions/{key}`) resolves through the SPA fallback, and the retired IA-v1 paths `/settings`, `/gateway`, and `/about` canonicalise to `/` — those surfaces live in the Settings modal now. The JSON API SHALL serve: `GET /api/sessions` and `POST /api/sessions` (create, with optional `prompt` field and a required `agent` field naming the target agent), `GET /api/sessions/{key}`, `POST /api/sessions/{key}/message`, `POST /api/sessions/{key}/close`, `POST /api/sessions/{key}/switch`, `POST /api/sessions/{key}/pending/{pending_id}/remove` (remove a not-yet-started submission), `POST /api/sessions/{key}/pending/{pending_id}/move` (reorder a not-yet-started submission within its own disposition group; body `to_index`), `GET /api/summary`, `POST /api/permissions/{request_id}/answer`, `GET /api/settings`, `GET /api/router`, `GET /api/about`, `POST /api/sessions/{key}/model` (mid-session model switch), the agent catalog `GET /api/agents` (each configured agent plus the built-in native kernel, with id, display name, reachability, optional cause and version), `GET /router/api/presets` (read-only preset table), `GET/POST /api/auth/login`, `GET /api/auth/me`, `POST /api/auth/logout`, the project APIs `GET /api/projects` and `POST /api/projects` (register), `POST /api/projects/reorder`, `POST /api/projects/{id}/remove`, `GET /api/projects/{id}/branch`, `GET /api/fs/browse-dirs` (lazy directory listing for the folder picker, scoped to the server's work directory — the configured work dir of the default agent kind, falling back to the WebUI process working directory; an explicit `root` query parameter overrides the default), `POST /api/sessions/{key}/archive` (archive a session), `POST /api/sessions/{key}/restore` (restore an archived session), `GET /api/archive` (list archived sessions with expiry info), and `GET /ws` (WebSocket session stream). Project and session mutations are POST-only and carry the same posture as the existing session APIs. The provider management cluster under `/router/api/*` (providers, model aliases, defaults, presets, model fetch) SHALL be fulfilled by the WebUI backend from the core-owned provider store over the core channel, never by proxying the router process; the route names are retained for compatibility. Without a reachable core these routes SHALL fail honestly (503) and SHALL NOT serve a stale snapshot. The JSON admin API `/api/admin/*` (status, events, services, login, logout, update, update/dry-run, update/dev, rollback, restart) is always mounted: without a control-plane adapter its reads report `adapter_ok: false` and its mutations return 503 (honest degradation). `GET /health` returns the literal `ok`. All browser assets the UI needs to render — styles, fonts, Web Awesome, markdown rendering, and syntax highlighting — are self-hosted under `/assets/*`; the UI SHALL NOT depend on an external CDN at render time. Navigation SHALL only link to routes this surface serves.

`GET /api/fs/browse-dirs` SHALL honour a path round-trip contract: the `path` echoed in a listing response SHALL be accepted verbatim as the `path` of a subsequent request for that same directory, and request paths that mix `/` and `\` separators SHALL resolve to the same directory. The echoed path SHALL NOT carry a Windows verbatim (`\\?\`) prefix.

Projects SHALL be identified on the wire by a stable `project_id` (`proj-<12hex>`, deterministically derived from the canonicalised path), not by the raw path string; the path itself SHALL NOT appear in session or project API request/response bodies as an identifier, though it may be included as display metadata.

The session payloads the workbench observes — the focused session in `GET /api/summary` and `GET /api/sessions/{key}` — SHALL carry the session's pending submissions in delivery order, each with its stable id, text, position, disposition (`staging` | `turn`) and priority flag.

#### Scenario: dashboard route

- **WHEN** a browser requests `/`
- **THEN** the SPA workbench renders, listing registered projects in the project rail, the Inbox grouping for sessions with no project, the History (archive) group, and the selected project's sessions

#### Scenario: session deep link still resolves

- **WHEN** a bookmarked `/sessions/{key}` is requested
- **THEN** the SPA fallback serves the shell and the client router renders the session's detail rather than 404, so links made before this change keep working

#### Scenario: admin cluster requires adapter

- **WHEN** the WebUI starts without a control-plane adapter
- **THEN** `/api/admin/*` reads report `adapter_ok: false` and mutations return 503

#### Scenario: router data reflects live state

- **WHEN** a provider is renamed through the provider management surface and the browser then requests `GET /api/router`
- **THEN** the response lists the new provider name without a WebUI restart

#### Scenario: router mutations unavailable without secret

- **WHEN** the WebUI has no reachable core and a mutation is posted to `/router/api/providers`
- **THEN** the response is 503

#### Scenario: agents catalog lists configured agents and native

- **WHEN** the browser requests `GET /api/agents`
- **THEN** the response lists one entry per configured agent (`id` = the config key, `display` = its display name) plus one entry for the native kernel (`id = "native"`), each with `reachable`, an optional `cause` when unreachable, and an optional `version`; no `driver` or backend-implementation field appears in the payload

#### Scenario: create session requires an explicit agent

- **WHEN** `POST /api/sessions` is called without an `agent` field, or with a legacy `backend` value (`"acp"`, `"acp:<slug>"`, or null)
- **THEN** the request is rejected with 400 naming the `agent` field as required, and no session is created

#### Scenario: project id is the wire identifier

- **WHEN** a session is created with `project_id = "proj-abc123def456"` and later listed
- **THEN** the session and project payloads reference the project by that `project_id`, not by the raw directory path

#### Scenario: agent defaults read and set

- **WHEN** the browser requests `GET /api/agent-defaults` or `PUT /api/agent-defaults`
- **THEN** the response is 404 — this endpoint no longer exists. Default provider and model are managed through the router admin providers surface, and the default agent is remembered per project, not through a global agent-defaults endpoint.

#### Scenario: no external asset fetch

- **WHEN** any page is rendered
- **THEN** every stylesheet, script, and font it requests resolves under `/assets/*` and no request targets an external host

#### Scenario: navigation targets exist

- **WHEN** every navigation link in the rendered shell is requested
- **THEN** each resolves to a route served by this surface, including SPA client routes resolved through the fallback

#### Scenario: browse-dirs defaults to the server work directory

- **WHEN** `GET /api/fs/browse-dirs` is called with no `root` parameter
- **THEN** the listing is rooted at the server's configured work directory (or the process working directory when none is configured), and the response `path` echoes that directory

#### Scenario: browse-dirs path round-trips on expand

- **WHEN** the `path` echoed by a directory listing is sent back unchanged as the `path` of a request for one of its subdirectories (client-side join of the echoed parent and a child name)
- **THEN** the response is 200 with that subdirectory's entries, including on Windows where canonicalised paths would otherwise carry a `\\?\` prefix

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
- **THEN** the response is 400 with an out-of-scope error, and no directory content outside the allowed roots is disclosed

#### Scenario: browse-dirs accepts an allowed root

- **WHEN** `allowed_roots` contains a directory and a request carries that directory as the explicit `root` (or a subpath of it as `path`)
- **THEN** the listing succeeds for that scope

#### Scenario: unconfigured allowed_roots preserves current behavior

- **WHEN** `allowed_roots` is absent or empty
- **AND** `GET /api/fs/browse-dirs` is called with an explicit `root` that exists
- **THEN** the listing succeeds — the whitelist is opt-in, and the default fallback root (work dir / process cwd) remains the no-`root` scope

#### Scenario: project registration rejects a path outside allowed roots

- **WHEN** `allowed_roots` is configured
- **AND** `POST /api/projects` is called with a body path that resolves outside every allowed root
- **THEN** the response is 400 with an out-of-scope error and the project is not registered

#### Scenario: project registration accepts a path inside allowed roots

- **WHEN** `allowed_roots` is configured
- **AND** `POST /api/projects` is called with a path inside an allowed root
- **THEN** the project registers as before

#### Scenario: project registration without allowed_roots is unchanged

- **WHEN** `allowed_roots` is absent or empty
- **AND** `POST /api/projects` is called with an existing directory path
- **THEN** the project registers as before (existence checks only)

#### Scenario: pending submissions ride in the session payload

- **WHEN** the browser requests a session payload while submissions are staged during spawn or queued behind a running turn
- **THEN** the payload lists them in delivery order with id, text, position, disposition and priority

#### Scenario: pending submission removal and reorder are served

- **WHEN** `POST /api/sessions/{key}/pending/{pending_id}/remove` or `.../move` is called for a submission that has not started
- **THEN** the request succeeds and a subsequent session payload reflects the new pending list

#### Scenario: managing a started submission is a typed rejection

- **WHEN** either pending endpoint is called for a submission whose turn already started
- **THEN** the response is a 4xx typed rejection stating that it is already running, and the in-flight turn is unaffected

#### Scenario: queue overflow is a visible rejection

- **WHEN** `POST /api/sessions/{key}/message` is called while the session's spawn-window staging queue is at its cap
- **THEN** the response is a 4xx rejection naming the cap, and the response does not report the submission as accepted

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

### Requirement: 鉴权开关（auth）与凭据自动引导

WebUI SHALL 提供 `[watchdog.webui] auth` 配置开关，默认 `true`。
开关为 `true` 时，鉴权门 SHALL 恒在：凭据文件缺失时按优先级自动引导——
`SEBAS_WEBUI_TOKEN`（单字段登录密钥，SHA-256 摘要落盘）→
`SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD`（密码注入）→ 自动生成随机
密码并以醒目日志打印一次（用户名 `admin`，仅落 PBKDF2 哈希）；引导后
`/api/*`、`/router/api/*`、`/ws` 需要有效会话。开关为 `false` 时，即使
凭据文件存在，SHALL 对所有路由（含静态资源）完全放行，不要求登录且不
触发引导；`GET /api/auth/me` SHALL 报告 `enabled: false`（前端据此不渲染
登录页）。`sebas webui-passwd` 在开关关闭时仍可管理凭据（为重新启用做
准备），改密 SHALL 保留已引导的 token 摘要，但不产生任何强制登录效果。

#### Scenario: 默认打开且凭据存在

- **WHEN** 配置未写 `auth` 且凭据文件存在
- **THEN** 未带会话的 `/api/summary` 请求返回 401，行为与无开关时一致

#### Scenario: 默认打开且凭据缺失自动生成

- **WHEN** 配置未写 `auth`、凭据文件不存在、且未设凭据环境变量
- **THEN** 首次启动自动生成随机凭据（用户名 `admin`），密码打印到日志一次
- **AND** 未带会话的 `/api/summary` 请求返回 401（登录门开箱即用）

#### Scenario: token 环境变量注入

- **WHEN** 凭据文件不存在且 `SEBAS_WEBUI_TOKEN` 非空
- **THEN** 该 token 以 SHA-256 摘要写入凭据文件，单字段登录可用

#### Scenario: 测试环境关闭

- **WHEN** 配置设置 `watchdog.webui.auth = false` 且凭据文件存在
- **THEN** 未带任何会话的 `/api/summary` 请求返回 200，全部路由免登录
- **AND** `GET /api/auth/me` 返回 `{"enabled": false, "authenticated": false}`

#### Scenario: 关闭后重新打开立即生效

- **WHEN** 开关从 `false` 改回 `true` 并重启 webui
- **THEN** 已存在的凭据立即恢复强制登录，无需重建凭据文件

### Requirement: 单字段登录（token 或密码）

`POST /api/auth/login` SHALL 接受单字段 `{"secret"}`（登录 token 或账户
密码，服务端自动识别），并保持兼容旧格式 `{"username", "password"}`。
成功即建立会话 cookie；失败统一 401，限速策略不变（按来源 IP）。

#### Scenario: token 登录

- **WHEN** `SEBAS_WEBUI_TOKEN` 引导的 token 通过 `{"secret"}` 提交
- **THEN** 登录成功并返回会话 cookie，响应携带服务端账户名

#### Scenario: 密码单字段登录

- **WHEN** 引导生成的随机密码通过 `{"secret"}` 提交
- **THEN** 登录成功（密码与 token 由服务端自动识别，无需用户名字段）

#### Scenario: 旧格式保持兼容

- **WHEN** `{"username", "password"}` 提交到 `/api/auth/login`
- **THEN** 行为与单字段形态一致（校验通过即建立会话）

#### Scenario: 登录页单字段

- **WHEN** 前端渲染登录门
- **THEN** 登录表单只有一个凭据输入框（密码或 token 通吃），401 就地
  提示「凭据错误」

### Requirement: 非 loopback bind 与开关联动

当 `watchdog.webui.host` 非 loopback 时，webui SHALL 仅在
`auth = true`（或缺省）且登录凭据存在时才允许绑定启动；凭据缺失时先经
自动引导（见「鉴权开关（auth）与凭据自动引导」），引导后凭据即存在，
因此该状态下 SHALL 放行启动。开关关闭时无论凭据是否存在，SHALL 拒绝
非 loopback bind（防止误关开关叠加公网暴露）。

#### Scenario: 开关关闭拒绝公网 bind

- **WHEN** 配置同时设置 `auth = false` 与 `host = "0.0.0.0"`
- **THEN** `sebas webui` 以配置错误退出，不绑定端口

#### Scenario: 开关打开且凭据存在允许公网 bind

- **WHEN** 配置设置 `auth = true`（或缺省）、`host = "0.0.0.0"`，
  且凭据文件存在（含自动引导生成）
- **THEN** `sebas webui` 正常绑定并在日志中提示已启用登录鉴权

### Requirement: Optional admin authentication

The `/api/admin/*` control-plane surface SHALL require its own admin session
(a separate cookie from the main login) when `SEBAS_CONTROL_SECRET` is
configured: unauthenticated admin API requests get a JSON 401 (the HTML
`/admin/*` pages are retired; unauthenticated page paths fall through the
SPA fallback). A successful admin login sets an HttpOnly, SameSite=Lax
cookie with a 24 h TTL, and login attempts are rate-limited to 5 per 30 s.
When no control secret is configured, admin reads are loopback-only and
mutations report the control plane as disconnected. The main session APIs
are governed by the「鉴权开关（auth）与凭据自动引导」requirement, not this one.

#### Scenario: password-gated admin API

- **WHEN** the admin credential is set and an unauthenticated request hits
  `/api/admin/status`
- **THEN** the response is a JSON 401 (not a redirect)

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

The cross-project session list SHALL render one row per known session (encoded key, chat and thread ids, session id, status, phase, relative last-active), active-first, and SHALL be reachable from the workbench rather than from primary navigation. The session list SHALL exclude archived sessions — those are served by `GET /api/archive`. Selecting a session in the rail, opening its `/sessions/{key}` deep link, or posting `/switch` SHALL focus that session in place — a display pointer only that never changes message routing — and `switch` returns the redirect target or 404 for an unknown key. There SHALL be no separate per-session detail surface: the workbench renders the focused session. Switching the displayed project SHALL NOT alter the focused session pointer. The rail's current-session marker SHALL be derived from the focused-session pointer, not from the browser location.

The focused-session pointer SHALL be the single source of truth for the workbench composer's follow-up vs creation mode: with a focused session the composer targets that session; without one the composer is in creation mode. Any path that focuses a session (switch endpoint, deep-link visit, placeholder creation) SHALL leave the composer able to submit a follow-up message to that session without further operator action.

#### Scenario: focus is cosmetic

- **WHEN** the user focuses session B in the WebUI while session A is active
- **THEN** subsequent Feishu messages still route per the router's own session mapping, unchanged

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

The WebUI settings SHALL provide a provider management page backed by the core-owned
provider store (see `provider-management`), reached through the WebUI's
`/router/api/providers*` surface. It SHALL support: listing providers with name,
preset-or-custom mark, base URL slots, key-configured state, and each model entry's
capability tags; creating a provider either from a preset or as custom; editing an
existing provider; and deleting a provider. Fetching a provider's official model list is
specified separately by the `Fetch models from the provider's official base URL`
requirement, not here.

The model list SHALL be a list of entries the operator may add to and remove from freely,
each entry carrying an id and its capability tags (`text` implicit, with `vision`,
`audio`, and `video` selectable). The list SHALL be editable for preset-derived and custom
providers alike.

Creating from a preset SHALL require only the preset choice, the API key, and optionally
the model entries; the preset's base URLs SHALL render as read-only code-owned values.
Creating a custom provider SHALL require the same inputs plus an instance name, a base URL,
and a wire protocol. Every input beyond that minimum — the remaining base URL slots, the
model rename map, and the preset's inherited `api_key_env` — SHALL live in an Advanced
disclosure that is collapsed by default. `api_key_env` SHALL NOT be a user input field; a
preset's env name remains only a fallback key source when no plaintext key is stored.

Secret inputs SHALL never be pre-filled, and an empty secret submit SHALL preserve the
stored key. Mutations SHALL surface their errors (duplicate, validation, unavailable) in
the page. Choosing an option inside any select control of the provider editor SHALL NOT
close the editor.

#### Scenario: preset-derived provider shows read-only URLs

- **WHEN** the user opens provider `glm` (preset-derived) for editing
- **THEN** the base URL fields render the preset's code-owned values as read-only with a
  follow-the-code indication, while the API key, default model, model entries, and their
  capability tags remain editable

#### Scenario: create from preset

- **WHEN** the user creates a provider from preset `glm` entering an API key and two model
  entries
- **THEN** the create request carries the preset selection, the key, and the model entries
  with capability tags, but no base URL fields, and the provider appears in the list

#### Scenario: custom provider full editing

- **WHEN** the user creates a custom provider filling a single base URL
- **THEN** the create succeeds and the edit form exposes all three slots — the two extras
  under the Advanced disclosure — for later adjustment

#### Scenario: probe from the page

- **WHEN** the user runs a fetch on a provider with a usable base URL
- **THEN** the page shows the returned model ids without leaving the page

#### Scenario: delete reflects immediately

- **WHEN** the user deletes provider `alpha` from the page
- **THEN** the provider disappears from the list without a page reload

#### Scenario: preset instance name defaults without input

- **WHEN** the user creates from preset `glm` without opening the Advanced disclosure
- **THEN** the provider is stored under the name `glm`, and renaming it is the only path to
  a second instance of the same preset

#### Scenario: custom provider minimal input

- **WHEN** the user creates a custom provider entering an instance name, a base URL, and a
  wire protocol
- **THEN** the create succeeds without the operator opening the remaining URL slots or the
  model rename map

#### Scenario: advanced section collapsed by default

- **WHEN** the provider editor opens in either mode
- **THEN** the remaining base URL slots, the model rename map, and the inherited
  `api_key_env` are behind a collapsed disclosure, and none of them is submitted as a
  user-entered value unless the operator opens it

#### Scenario: multiple model entries with capability tags

- **WHEN** the user adds several model entries to a provider and marks one as `vision`
- **THEN** the list preserves order, that entry carries `vision` alongside the implicit
  `text`, and the others carry `text` alone

#### Scenario: model list is editable for a preset-derived provider

- **WHEN** the user adds a model entry to a preset-derived provider
- **THEN** the entry is stored on that provider without altering the preset's code-table
  data

#### Scenario: selecting an option does not close the editor

- **WHEN** the user picks a preset, a protocol, or a model option inside the provider
  editor
- **THEN** the editor stays open and the chosen value is retained

#### Scenario: secret is preserved on empty submit

- **WHEN** the user edits a provider and submits with the API key field left empty
- **THEN** the stored key is unchanged and no key material was rendered

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
provider's official base URL currently serves. The action SHALL be available for
preset-derived and custom providers alike, and SHALL be hidden for a provider with no
usable base URL. Running it SHALL persist nothing: the returned ids are shown as a
result list, and an id joins the provider's model list only when the operator picks it,
which is an ordinary edit. Fetched models SHALL start with no capability tags beyond the
implicit text capability, and their parameters SHALL be shown as locally resolved or
unknown rather than inferred from the id. Failures SHALL be reported with the sanitized
reason and SHALL NOT be presented as an empty successful list.

#### Scenario: fetch lists the official model ids

- **WHEN** the operator runs fetch on a preset-derived provider whose base URL serves a model list
- **THEN** the result list shows the returned ids, and the provider's stored data is unchanged until the operator picks one

#### Scenario: picking a fetched model edits the list

- **WHEN** the operator picks a fetched id
- **THEN** that id is added to the provider's model list with the implicit text capability and no invented parameters

#### Scenario: no base URL means no fetch entry

- **WHEN** a provider has no usable base URL
- **THEN** the fetch action is not rendered for it

#### Scenario: failure is reported honestly

- **WHEN** the upstream fetch fails
- **THEN** the surface shows the sanitized reason, and does not display an empty list as if the provider offered no models

### Requirement: Services 分区与 router 状态归属
Services 分区 SHALL 以 watchdog 受管子进程为唯一数据源：调用 `GET /api/admin/services` 获取受管服务表，渲染每个进程的 name / desired / actual status / uptime_secs / 最近错误（由 `/api/admin/events` 提供，无事件则不渲染错误行）。受管服务名固定为 `core` / `webui` / `router` / `im`（IM 在配置未启用时不出现；产品对外名称保留「飞书」由前端做 i18n）。core 为恒启动服务：其行 SHALL 仅呈现状态与 restart 入口，SHALL NOT 渲染 enable/disable 按钮（enable-core-by-default）。无 watchdog adapter 时 SHALL 显式呈现 `adapter_ok: false` 横幅、不暴露 enable/disable/restart 按钮；该形态下 `/api/admin/services` 返回空数组且后端响应携带 `adapter_ok: false`。router 的运行状态（desired / actual / uptime）SHALL 仅由本分区呈现；Models 分区 SHALL NOT 呈现 router 网关总览、listen / debug / auth 或任何 router 运行状态，provider 管理面与 router 运行状态在产品语义上分离。

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

- **WHEN** 操作员对辅助服务（如 `router`）点击 disable（前提：confirm 弹窗已确认）
- **THEN** 前端 POST `/api/admin/services/router/disable` 收到 200；列表行刷新；core 行不渲染 enable/disable 按钮

#### Scenario: restart 操作

- **WHEN** 操作员对 `core` 点击 restart
- **THEN** 前端走 `Admin actions via control plane` 既有 restart-core 路径；成功后列表行 uptime 重置

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

#### Scenario: both sides of the conversation are in the payload

- **WHEN** the browser requests a session in which the operator submitted messages across several turns
- **THEN** the payload carries one ordered entry sequence containing both the submissions and the agent's output, each entry stating kind and element_type

#### Scenario: retired fields are gone

- **WHEN** a session payload is returned
- **THEN** it carries no single-prompt field and no agent-output-only list, and the conversation is available only as the ordered entry sequence

#### Scenario: empty session is not an error

- **WHEN** a session has no transcript entries yet
- **THEN** the payload returns an empty entry sequence with a success status

### Requirement: 会话创建携带 mode

`POST /api/sessions` SHALL 接受可选 `mode` 字段，词汇为 `ask / edit / allow / auto`（与节点链路 `SessionMode` 一致）。缺省或 null 表示"agent 默认行为"，wire 上不携带 mode。未知词汇 SHALL 返回 400 指出非法值，SHALL NOT 静默降级为默认。`mode` 与 `agent`/`model` 一样在创建时记入会话映射；0-turn 占位会话 SHALL 记住请求的 mode，并在首条消息触发 spawn 时应用。远端节点上的 0-turn 占位仍按既有规则如实拒绝。

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

会话 dashboard SHALL 展示当前会话的 mode（含远端会话已有的 desired/effective 呈现），并提供切换入口（下拉/菜单），提交后走中途切换端点。mode 显示对远端节点会话沿用 effective/desired 差异化呈现（effective 缺失时只显 desired）。

#### Scenario: composer 创建表单的 mode 选择

- **WHEN** 操作者在创建模式展开表单
- **THEN** 表单提供 mode 下拉，缺省项为"agent 默认"（不发送 mode 字段）

#### Scenario: 会话头部切换 mode

- **WHEN** 操作者在会话头部选择另一个 mode
- **THEN** 前端提交 `POST /api/sessions/{key}/mode`；成功后头部 mode 更新，失败显示非致命错误且保持原显示

### Requirement: SessionBackend seam 承载 mode

`SessionBackend` 的 spawn / placeholder / 切换方法 SHALL 携带 mode 维度（与 agent/model/node 同构）：进程内后端把 mode 交给本机执行体映射；分离部署经核心通道 Spawn 帧携带 mode；远端放置把 mode 交给节点投影。 seam 的默认实现对 mode 的降级 SHALL 如实（不支持 mode 的后端不假装生效）。

#### Scenario: 分离部署 wire 携带 mode

- **WHEN** webui 与 core 分离部署、创建会话带 mode
- **THEN** 核心通道 Spawn 帧携带该 mode，core 侧按同一映射语义放置

#### Scenario: 不支持 mode 的执行体如实回报

- **WHEN** mode 发给无法生效它的执行体（如 native 内核、未声明 mode 能力的通用 ACP agent）
- **THEN** 创建成功但该执行体不声称 mode 生效；能回报 effective 的位置如实回报空
