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

#### Scenario: router data reflects live state

- **WHEN** a provider is renamed through the router admin API and the
  browser then requests `GET /api/router`
- **THEN** the response lists the new provider name without a WebUI restart

#### Scenario: router mutations unavailable without secret

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
