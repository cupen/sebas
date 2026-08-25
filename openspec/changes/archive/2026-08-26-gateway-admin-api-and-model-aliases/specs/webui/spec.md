## MODIFIED Requirements

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
