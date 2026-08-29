## MODIFIED Requirements

### Requirement: HTTP route surface

The WebUI SHALL serve: `GET /` (dashboard overview), `GET /sessions` (full
list), `GET /sessions/partial` (table fragment), `GET /sessions/{key}`
(detail), `GET /settings`, `GET /gateway`, `GET /about`, `GET /events`
(SSE), `GET /health` (literal `ok`), `GET /static/*` (static assets), and
the session APIs `POST /api/sessions` (create), `POST
/api/sessions/{key}/message`, `POST /api/sessions/{key}/close`, `POST
/api/sessions/{key}/switch`. The `/admin/*` cluster (status, events,
update, update/dry-run, update/dev, rollback, restart, services, login,
logout) is mounted only when a control-plane adapter is configured.
Templates are compiled into the binary. All browser assets the UI needs to
render — styles, fonts, vendored htmx, markdown rendering, and syntax
highlighting — are served from disk under `/static/*`; the UI SHALL NOT
depend on an external CDN at render time. Navigation SHALL only link to
routes this surface serves.

#### Scenario: dashboard route

- **WHEN** a browser requests `/`
- **THEN** the overview renders active/dormant/spawning counts, uptime, and
  recent sessions

#### Scenario: admin cluster requires adapter

- **WHEN** the WebUI starts without a control secret
- **THEN** `/admin/*` routes either 404 or render with "control plane not
  connected", and mutations return 503

#### Scenario: no external asset fetch

- **WHEN** any page is rendered
- **THEN** every stylesheet, script, and font it requests resolves under
  `/static/*` and no request targets an external host

#### Scenario: navigation targets exist

- **WHEN** every navigation link in the rendered shell is requested
- **THEN** each resolves to a route served by this surface
