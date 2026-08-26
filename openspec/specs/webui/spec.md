# webui Specification

## Purpose
Defines the local web dashboard: its HTTP route surface, the local-only
security baseline (loopback binding, optional admin password, mutation
guards), the session dashboard and its focus/close semantics, the detached
behavior of the watchdog-spawned instance, and the admin actions proxied to
the watchdog control plane.

## Requirements

### Requirement: HTTP route surface

The WebUI SHALL serve: `GET /` (dashboard overview), `GET /sessions` (full
list), `GET /sessions/partial` (table fragment), `GET /sessions/{key}`
(detail), `GET /settings`, `GET /gateway` (live gateway view), `GET /about`,
`GET /events` (SSE), `GET /health` (literal `ok`), `GET /static/*` (static
assets), and the session APIs `POST /api/sessions` (create), `POST
/api/sessions/{key}/message`, `POST /api/sessions/{key}/close`, `POST
/api/sessions/{key}/switch`. The `/admin/*` cluster (status, events,
update, update/dry-run, update/dev, rollback, restart, services, login,
logout) is mounted only when a control-plane adapter is configured. The
gateway editing cluster — `GET /gateway` plus its fragment routes and
POST-only JSON mutation routes under `/gateway/api/*` (provider and model
alias CRUD, provider probe, reload) — is functional only when a control
secret is configured; without it `GET /gateway` renders a degraded
read-only view and the mutation routes return 503. Gateway data on these
pages is fetched live from the gateway admin API at request time (proxied
server-side by the WebUI backend with the control secret), not from a
startup snapshot. Templates are compiled into the binary; static assets
(styles, vendored htmx) are served from disk; syntax highlighting and
markdown rendering load from external CDNs.

#### Scenario: dashboard route

- **WHEN** a browser requests `/`
- **THEN** the overview renders active/dormant/spawning counts, uptime, and
  recent sessions

#### Scenario: admin cluster requires adapter

- **WHEN** the WebUI starts without a control secret
- **THEN** `/admin/*` routes either 404 or render with "control plane not
  connected", and mutations return 503

#### Scenario: gateway page reflects live state

- **WHEN** a provider is renamed through the gateway admin API and the
  browser then requests `GET /gateway`
- **THEN** the rendered page lists the new provider name without a WebUI
  restart

#### Scenario: gateway mutations unavailable without secret

- **WHEN** the WebUI runs without a control secret and a mutation is posted
  to `/gateway/api/providers`
- **THEN** the response is 503

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

The dashboard SHALL render one row per known session (encoded key, chat and
thread ids, session id, status, phase, relative last-active), active-first.
Visiting a session's detail page or posting `/switch` SHALL set the
webui-side focused session — a display pointer only that never changes
message routing — and `switch` returns the redirect target or 404 for an
unknown key.

#### Scenario: focus is cosmetic

- **WHEN** the user focuses session B in the WebUI while session A is
  active
- **THEN** subsequent Feishu messages still route per the router's own
  session mapping, unchanged

#### Scenario: switch unknown key

- **WHEN** `/api/sessions/{key}/switch` posts a key not in the map
- **THEN** the response is 404

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

### Requirement: Standalone detached semantics

The watchdog-spawned (standalone) WebUI SHALL operate on its own `RouterHandle`
restored from the state file with a throwaway session manager and the
outbound instruction channel deliberately dropped: session create, message
send, and close mutate only the local in-process state and never spawn a
real ACP session, send to Feishu, or affect the running core. The legacy
`run --webui` path instead shares the live router and manager with the WS
bridge.

#### Scenario: standalone message send is local

- **WHEN** the user sends a message through a standalone WebUI's session
  page
- **THEN** no ACP child is spawned and no Feishu message is sent; only the
  local mapping state changes

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
