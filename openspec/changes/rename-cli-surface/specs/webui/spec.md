## MODIFIED Requirements

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
`GET /api/router`, `GET /api/about`, the project APIs `GET /api/projects` and
`POST /api/projects` (register), `POST /api/projects/reorder`, `POST
/api/projects/{path}/remove`, `GET /api/projects/{path}/branch`, `GET
/api/fs/browse-dirs` (lazy directory listing for the folder picker, scoped
to a server-configured work root), `POST
/api/sessions/{key}/archive` (archive a session), `POST
/api/sessions/{key}/restore` (restore an archived session), `GET /api/archive`
(list archived sessions with expiry info), and `GET /ws`
(WebSocket session stream). Project and session mutations are POST-only and
carry the same posture as the existing session APIs. The router mutation
cluster — POST/PUT/DELETE under `/router/api/*` (provider and model-alias
CRUD, provider probe, reload) — is functional only when a control secret is
configured; without it the mutations return 503. Router data is fetched live
from the router admin API at request time (proxied server-side by the WebUI
backend with the control secret), not from a startup snapshot. The JSON admin
API `/api/admin/*` (status, events, services, login, logout, update,
update/dry-run, update/dev, rollback, restart) is always mounted: without a
control-plane adapter its reads report `adapter_ok: false` and its mutations
return 503 (honest degradation). `GET /health` returns the literal `ok`. All
browser assets the UI needs to render — styles, fonts, Web Awesome, markdown
rendering, and syntax highlighting — are self-hosted under `/assets/*`; the UI
SHALL NOT depend on an external CDN at render time. Navigation SHALL only link
to routes this surface serves.

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

#### Scenario: gateway page reflects live state

- **WHEN** a provider is renamed through the router admin API and the
  browser then requests `GET /api/router`
- **THEN** the response lists the new provider name without a WebUI restart

#### Scenario: gateway mutations unavailable without secret

- **WHEN** the WebUI runs without a control secret and a mutation is posted
  to `/router/api/providers`
- **THEN** the response is 503

#### Scenario: no external asset fetch

- **WHEN** any page is rendered
- **THEN** every stylesheet, script, and font it requests resolves under
  `/assets/*` and no request targets an external host

#### Scenario: navigation targets exist

- **WHEN** every navigation link in the rendered shell is requested
- **THEN** each resolves to a route served by this surface, including SPA
  client routes resolved through the fallback

#### Scenario: directory browser rejects parent escape

- **WHEN** `GET /api/fs/browse-dirs?path=/etc&root=/home/user` is called
- **AND** the path is canonicalised and found to navigate outside the operator's home directory
- **THEN** the response is 400 with an error message

#### Scenario: archive endpoint exists

- **WHEN** `POST /api/sessions/{key}/archive` is called
- **THEN** the session is archived, moved from the active session list, and the response confirms the action

#### Scenario: archive list endpoint

- **WHEN** `GET /api/archive` is called
- **THEN** the response lists all archived sessions with their original project, archive timestamp, and expiry

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

#### Scenario: gateway mutation is post-only and origin-checked

- **WHEN** a GET hits `/router/api/providers` or a router mutation POST
  carries a non-loopback origin without a valid CSRF token
- **THEN** the response is 405 (GET) or 403 (foreign origin)

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

The WebUI crate SHALL access sessions through a backend abstraction rather than a
concrete `DispatchHandle`, in the same shape as the existing admin adapter, so the
crate carries no knowledge of whether the core is in-process or across a socket.
The crate SHALL NOT depend on the sebas binary crate to obtain a backend; the
binary crate SHALL supply the implementation at startup.

#### Scenario: WebUI is testable without a core

- **WHEN** the WebUI's route tests run
- **THEN** they drive routes through a fake backend, with no ACP child, no socket,
  and no state file

#### Scenario: no backend leaks into templates

- **WHEN** a page is rendered
- **THEN** which backend is in use is not visible in the markup except where the
  channel's degradation contract requires stating that the core is not connected

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
