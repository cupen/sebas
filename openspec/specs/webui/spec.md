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
`GET /api/gateway`, `GET /api/about`, the project APIs `GET /api/projects` and
`POST /api/projects` (register), `POST /api/projects/reorder`, `POST
/api/projects/{path}/remove`, `GET /api/projects/{path}/branch`, `GET
/api/fs/browse-dirs` (lazy directory listing for the folder picker, scoped
to a server-configured work root), `POST
/api/sessions/{key}/archive` (archive a session), `POST
/api/sessions/{key}/restore` (restore an archived session), `GET /api/archive`
(list archived sessions with expiry info), and `GET /ws`
(WebSocket session stream). Project and session mutations are POST-only and
carry the same posture as the existing session APIs. The gateway mutation
cluster — POST/PUT/DELETE under `/gateway/api/*` (provider and model-alias
CRUD, provider probe, reload) — is functional only when a control secret is
configured; without it the mutations return 503. Gateway data is fetched live
from the gateway admin API at request time (proxied server-side by the WebUI
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

- **WHEN** a provider is renamed through the gateway admin API and the
  browser then requests `GET /api/gateway`
- **THEN** the response lists the new provider name without a WebUI restart

#### Scenario: gateway mutations unavailable without secret

- **WHEN** the WebUI runs without a control secret and a mutation is posted
  to `/gateway/api/providers`
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

The standalone WebUI SHALL refuse to start when `watchdog.webui.host` is
not a loopback address, returning a configuration error. The legacy
`run --webui` path binds hard-coded `127.0.0.1`. Default bind is
`127.0.0.1:9797`.

#### Scenario: non-loopback refused

- **WHEN** the config sets `watchdog.webui.host = "0.0.0.0"`
- **THEN** `sebas webui` exits with a configuration error rather than
  binding

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
password is set, else 403. Gateway mutation routes under `/gateway/api/*`
follow the same posture: POST-only with the same origin check, and the
WebUI forwards them to the gateway admin API server-side with the control
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

- **WHEN** a GET hits `/gateway/api/providers` or a gateway mutation POST
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
construct its own `RouterHandle`, restore session state from the state file, or
hold a throwaway session manager. Session create, message send, and close SHALL
be requests to the core that spawn real ACP sessions and take effect in the
running core. The in-process `run --webui` path SHALL use an equivalent
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
  `run --webui`
- **THEN** session data and the availability of session controls are equivalent,
  differing only in which backend implementation serves them

### Requirement: Session backend seam

The WebUI crate SHALL access sessions through a backend abstraction rather than a
concrete `RouterHandle`, in the same shape as the existing admin adapter, so the
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

### Requirement: Admin actions via control plane

Admin mutations SHALL proxy to the watchdog over the control RPC using the
`SEBAS_CONTROL_SECRET`, attributed to a local CLI actor — which executes
directly without a confirmation round-trip. Actions: update (release),
update dry-run, update dev, rollback, restart core. When no adapter is
configured (secret absent), mutations return 503 "control plane not
connected".

#### Scenario: restart via admin

- **WHEN** the admin clicks restart on `/admin/update`-style pages with the
  control secret present
- **THEN** the watchdog receives `RestartCore` and restarts the core; the
  WebUI itself stays up

#### Scenario: no control plane

- **WHEN** the standalone WebUI has no `SEBAS_CONTROL_SECRET`
- **THEN** admin mutation buttons return 503

### Requirement: Watchdog lifecycle ownership

The WebUI SHALL be spawned by the watchdog as a separate process (with the
control secret) by default — `[watchdog.webui] enabled` defaults to `true`
unless explicitly set to `false` — and SHALL survive core restarts. The
WebUI SHALL bind to `127.0.0.1:9797` by default; port conflict with a
legacy `run --webui` (or any other process) is resolved by kernel-level
bind atomicity — the first to bind wins, the second bind fails with a
distinct exit code.

#### Scenario: single owner

- **WHEN** the watchdog-spawned WebUI is running and a legacy
  `run --webui` is attempted
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
