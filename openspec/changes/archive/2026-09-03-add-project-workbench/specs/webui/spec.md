## MODIFIED Requirements

### Requirement: HTTP route surface

The WebUI SHALL serve `GET /` as the SPA shell for the project workbench and
`GET /assets/*` for its built styles, scripts, and fonts. Any other
browser-facing GET (for example `/sessions/{key}`) resolves through the SPA
fallback, and the retired IA-v1 paths `/settings`, `/gateway`, and `/about`
canonicalise to `/` — those surfaces live in the Settings modal now. The JSON
API SHALL serve: `GET /api/sessions` and `POST /api/sessions` (create), `GET
/api/sessions/{key}`, `POST /api/sessions/{key}/message`, `POST
/api/sessions/{key}/close`, `POST /api/sessions/{key}/switch`, `GET
/api/summary`, `POST /api/permissions/{request_id}/answer`, `GET /api/settings`,
`GET /api/gateway`, `GET /api/about`, the project APIs `GET /api/projects` and
`POST /api/projects` (register), `POST /api/projects/reorder`, `POST
/api/projects/{path}/remove`, `GET /api/projects/{path}/branch`, and `GET /ws`
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
  project rail, the origin-named grouping for sessions with no project, and
  the selected project's sessions

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
