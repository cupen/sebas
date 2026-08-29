## MODIFIED Requirements

### Requirement: HTTP route surface

The WebUI SHALL serve `GET /` as the project workbench, plus: `GET /projects`
(project list), `GET /projects/{id}` (project view), `GET
/projects/{id}/sessions/{key}` (session turn stream), `GET
/projects/{id}/sessions/{key}/stream` (stream fragment), `GET /sessions`
(cross-project session list, subordinate to projects and not a primary
navigation entry), `GET /sessions/partial` (table fragment), `GET
/sessions/{key}` (detail), `GET /settings`, `GET /gateway`, `GET /about`, `GET
/events` (SSE), `GET /health` (literal `ok`), `GET /static/*` (static assets),
the project APIs `POST /api/projects` (register), `POST /api/projects/{id}/remove`,
`POST /api/projects/{id}/sessions` (start a session in the project), and the
session APIs `POST /api/sessions` (create), `POST
/api/sessions/{key}/message`, `POST /api/sessions/{key}/close`, `POST
/api/sessions/{key}/switch`. Project and session APIs are POST-only and carry
the same posture as the existing session APIs. The `/admin/*` cluster (status,
events, update, update/dry-run, update/dev, rollback, restart, services, login,
logout) is mounted only when a control-plane adapter is configured.
Templates are compiled into the binary. All browser assets the UI needs to
render — styles, fonts, vendored htmx, markdown rendering, and syntax
highlighting — are served from disk under `/static/*`; the UI SHALL NOT
depend on an external CDN at render time. Navigation SHALL only link to
routes this surface serves.

#### Scenario: dashboard route

- **WHEN** a browser requests `/`
- **THEN** the project workbench renders, listing registered projects, the
  origin-named grouping for sessions with no project, and the selected
  project's sessions

#### Scenario: session deep link still resolves

- **WHEN** a bookmarked `/sessions/{key}` is requested
- **THEN** the session's detail renders rather than 404, so links made before
  this change keep working

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

### Requirement: Session dashboard and focus semantics

The cross-project session list SHALL render one row per known session (encoded
key, chat and thread ids, session id, status, phase, relative last-active),
active-first, and SHALL be reachable from the workbench rather than from
primary navigation. Visiting a session's detail page or posting `/switch` SHALL
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
