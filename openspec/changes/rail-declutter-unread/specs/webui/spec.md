## MODIFIED Requirements

### Requirement: HTTP route surface

The WebUI SHALL serve `GET /` as the SPA shell for the project workbench and `GET /assets/*` for its built styles, scripts, and fonts. Any other browser-facing GET (for example `/sessions/{key}`) resolves through the SPA fallback, and the retired IA-v1 paths `/settings`, `/gateway`, and `/about` canonicalise to `/` — those surfaces live in the Settings modal now. The JSON API SHALL serve: `GET /api/sessions` and `POST /api/sessions` (create, with optional `prompt` field and a required `agent` field naming the target agent), `GET /api/sessions/{key}`, `POST /api/sessions/{key}/message`, `POST /api/sessions/{key}/close`, `POST /api/sessions/{key}/switch`, `POST /api/sessions/{key}/pending/{pending_id}/remove` (remove a not-yet-started submission), `POST /api/sessions/{key}/pending/{pending_id}/move` (reorder a not-yet-started submission within its own disposition group; body `to_index`), `GET /api/summary`, `POST /api/permissions/{request_id}/answer`, `GET /api/settings`, `GET /api/router`, `GET /api/about`, `POST /api/sessions/{key}/model` (mid-session model switch), `POST /api/sessions/{key}/mode` (mid-session permission-mode switch), the agent catalog `GET /api/agents` (each configured agent plus the built-in native kernel, with id, display name, reachability, optional cause and version), `GET /router/api/presets` (read-only preset table), `GET/POST /api/auth/login`, `GET /api/auth/me`, `POST /api/auth/logout`, the project APIs `GET /api/projects` and `POST /api/projects` (register), `POST /api/projects/reorder`, `POST /api/projects/{id}/remove`, `GET /api/projects/{id}/branch`, `GET /api/fs/browse-dirs` (lazy directory listing for the folder picker, scoped to the server's work directory — the configured work dir of the default agent kind, falling back to the WebUI process working directory; an explicit `root` query parameter overrides the default), `POST /api/sessions/{key}/archive` (archive a session), `POST /api/sessions/{key}/restore` (restore an archived session), `GET /api/archive` (list archived sessions with expiry info), and `GET /ws` (WebSocket session stream). Project and session mutations are POST-only and carry the same posture as the existing session APIs. The provider management cluster under `/router/api/*` (providers, model aliases, defaults, presets, model fetch) SHALL be fulfilled by the WebUI backend from the core-owned provider store over the core channel, never by proxying the router process; the route names are retained for compatibility. Without a reachable core these routes SHALL fail honestly (503) and SHALL NOT serve a stale snapshot. The JSON admin API `/api/admin/*` (status, events, services, login, logout, update, update/dry-run, update/dev, rollback, restart) is always mounted: without a control-plane adapter its reads report `adapter_ok: false` and its mutations return 503 (honest degradation). `GET /health` returns the literal `ok`. All browser assets the UI needs to render — styles, fonts, Web Awesome, markdown rendering, and syntax highlighting — are self-hosted under `/assets/*`; the UI SHALL NOT depend on an external CDN at render time. Navigation SHALL only link to routes this surface serves.

`GET /api/fs/browse-dirs` SHALL honour a path round-trip contract: the `path` echoed in a listing response SHALL be accepted verbatim as the `path` of a subsequent request for that same directory, and request paths that mix `/` and `\` separators SHALL resolve to the same directory. The echoed path SHALL NOT carry a Windows verbatim (`\\?\`) prefix.

Projects SHALL be identified on the wire by a stable `project_id` (`proj-<12hex>`, deterministically derived from the canonicalised path), not by the raw path string; the path itself SHALL NOT appear in session or project API request/response bodies as an identifier, though it may be included as display metadata.

The session payloads the workbench observes — the focused session in `GET /api/summary` and `GET /api/sessions/{key}` — SHALL carry the session's pending submissions in delivery order, each with its stable id, text, position, disposition (`staging` | `turn`) and priority flag.

#### Scenario: dashboard route

- **WHEN** a browser requests `/`
- **THEN** the SPA workbench renders, listing registered projects in the project rail, the History (archive) group, and the selected project's sessions

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
