## MODIFIED Requirements

### Requirement: HTTP route surface

The WebUI SHALL serve `GET /` as the SPA shell for the project workbench and
`GET /assets/*` for its built styles, scripts, and fonts. Any other
browser-facing GET (for example `/sessions/{key}`) resolves through the SPA
fallback, and the retired IA-v1 paths `/settings` and `/about`
canonicalise to `/` — those surfaces live in the Settings modal now. The
IA-v1 `/gateway` path is deleted outright (no route, no redirect): like
`/admin/*` it falls back to the workbench as an unknown path. The JSON
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

