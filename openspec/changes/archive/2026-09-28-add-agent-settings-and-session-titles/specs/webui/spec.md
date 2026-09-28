## MODIFIED Requirements

### Requirement: HTTP route surface

The WebUI SHALL serve `GET /` as the SPA shell for the project workbench and
`GET /assets/*` for its built styles, scripts, and fonts. Any other
browser-facing GET (for example `/sessions/{key}`) resolves through the SPA
fallback, and the retired IA-v1 paths `/settings`, `/gateway`, and `/about`
canonicalise to `/` — those surfaces live in the Settings modal now. The JSON
API SHALL serve: `GET /api/sessions` and `POST /api/sessions` (create, with
optional `prompt` field and a required `agent` field naming the target
agent), `GET /api/sessions/{key}`, `POST /api/sessions/{key}/message`,
`POST /api/sessions/{key}/cancel` (interrupt the session's in-flight turn
over the core channel), `POST /api/sessions/{key}/close`,
`POST /api/sessions/{key}/switch`,
`POST /api/sessions/{key}/pending/{pending_id}/remove` (remove a
not-yet-started submission), `POST /api/sessions/{key}/pending/{pending_id}/move`
(reorder a not-yet-started submission within its own disposition group; body
`to_index`), `GET /api/summary`, `POST /api/permissions/{request_id}/answer`,
`GET /api/settings`, `GET /api/about`, `POST /api/sessions/{key}/model`
(mid-session model switch), `POST /api/sessions/{key}/mode` (mid-session
permission-mode switch), `POST /api/sessions/{key}/activate` (start the
child in the background for a placeholder/dormant session), `GET
/api/sessions/{key}/approvals` (approval read model for restoring pending
review cards), `POST /api/sessions/{key}/label` (set the operator label),
the agent management cluster `GET /api/agents` (the built-in native kernel
plus every agents-store row — config-seeded or UI-created — with id, display
name, reachability, optional cause and version), `POST /api/agents` (create),
`PUT /api/agents/{id}` (update) and `DELETE /api/agents/{id}` (delete),
`GET /api/nodes` (remote execution node list),
the skills surface `GET/POST /api/skills` (store listing; sync trigger) and
`GET/DELETE /api/skills/{name}`, `GET /api/provider-presets` (read-only preset
table), `POST /api/auth/login` (the login page itself is rendered by the SPA),
`GET /api/auth/me`, `POST
/api/auth/logout`, the project APIs `GET /api/projects` and `POST
/api/projects` (register), `POST /api/projects/reorder`, `POST
/api/projects/{id}/remove`, `GET /api/projects/{id}/branch`, `GET
/api/fs/browse-dirs` (lazy directory listing for the folder picker, scoped to
the workspace root — the listing starts at the workspace root, and an
explicit `root` query parameter is honoured only inside it), `POST
/api/sessions/{key}/archive` (archive a session), `POST
/api/sessions/{key}/restore` (restore an archived session), `GET /api/archive`
(list archived sessions with expiry info), `GET /api/archive/{key}` (one
archived session's snapshot), and `GET /ws` (WebSocket session
stream). Project and session mutations are POST-only and carry the same
posture as the existing session APIs. The provider management cluster under
`/api/*` (`GET/POST /api/providers`, `PUT/DELETE /api/providers/{name}`,
`POST /api/providers/{name}/probe`, `GET /api/provider-defaults`, `POST
/api/model-aliases` (create/update one alias), `PUT/DELETE
/api/model-aliases/{alias}`; the alias table is read through the providers
projection rather than a dedicated list endpoint) SHALL be fulfilled by the WebUI backend from the
core-owned provider store over the core channel, never by proxying the router
process; the agent management cluster SHALL likewise be fulfilled from the
core-owned agents store over the core channel. The retired `/router/api/*` namespace (including `POST
/router/api/reload`) and the retired `GET /api/router` endpoint SHALL NOT be
served. Without a reachable core these routes SHALL fail honestly (503) and
SHALL NOT serve a stale snapshot. The JSON admin API `/api/admin/*` (status,
events, services, update, update/dry-run, update/dev, rollback, restart) is
always mounted: without a control-plane adapter its reads report
`adapter_ok: false` and its mutations return 503 (honest degradation). `GET
/health` returns the literal `ok`. All browser assets the UI needs to render
— styles, fonts, Web Awesome, markdown rendering, and syntax highlighting —
are self-hosted under `/assets/*`; the UI SHALL NOT depend on an external CDN
at render time. Navigation SHALL only link to routes this surface serves.

The WebUI SHALL enforce the workspace root (per the `workspace-root`
capability) on every project-directory surface: local project registration
SHALL accept only paths inside the workspace root; the project list SHALL
omit local projects outside it; and viewing or opening a session bound to a
local project outside it SHALL be rejected. Housekeeping writes on such
sessions (close, archive) SHALL remain available so out-of-scope sessions can
be cleaned up. A legacy `[watchdog.webui] allowed_roots` key SHALL be
ignored.

`GET /api/fs/browse-dirs` SHALL honour a path round-trip contract: the `path`
echoed in a listing response SHALL be accepted verbatim as the `path` of a
subsequent request for that same directory, and request paths that mix `/`
and `\` separators SHALL resolve to the same directory. The echoed path SHALL
NOT carry a Windows verbatim (`\\?\`) prefix.

Projects SHALL be identified on the wire by a stable `project_id`
(`proj-<12hex>`, deterministically derived from the canonicalised path), not
by the raw path string; the path itself SHALL NOT appear in session or
project API request/response bodies as an identifier, though it may be
included as display metadata.

The session payloads the workbench observes — the focused session in `GET
/api/summary` and `GET /api/sessions/{key}` — SHALL carry the session's
pending submissions in delivery order, each with its stable id, text,
position, disposition (`staging` | `turn`) and priority flag.

#### Scenario: dashboard route

- **WHEN** a browser requests `/`
- **THEN** the SPA workbench renders, listing registered projects in the
  project rail, the History (archive) group, and the selected project's
  sessions

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

- **WHEN** a provider is renamed through the provider management surface and
  the browser then requests `GET /api/providers`
- **THEN** the response lists the new provider name without a WebUI restart

#### Scenario: router mutations unavailable without secret

- **WHEN** the WebUI has no reachable core and a mutation is posted to
  `/api/providers`
- **THEN** the response is 503

#### Scenario: retired router surface is gone

- **WHEN** any request hits `/router/api/*` or `GET /api/router`
- **THEN** the response is 404

#### Scenario: agents catalog lists configured agents and native

- **WHEN** the browser requests `GET /api/agents`
- **THEN** the response lists one entry for the native kernel (`id =
  "native"`) plus one entry per agents-store row (`id` = the store row id —
  config-seeded or UI-created, `display` = its display name), each with
  `reachable`, an optional `cause` when unreachable, and an optional
  `version`; no `driver` or backend-implementation field appears in the
  payload

#### Scenario: agents CRUD mutations reach the store

- **WHEN** the browser creates, updates, or deletes an agent through
  `POST /api/agents`, `PUT /api/agents/{id}`, or `DELETE /api/agents/{id}`
- **THEN** the mutation commits to the agents store and a subsequent
  `GET /api/agents` reflects it without a WebUI or core restart

#### Scenario: create session requires an explicit agent

- **WHEN** `POST /api/sessions` is called without an `agent` field, or with a
  legacy `backend` value (`"acp"`, `"acp:<slug>"`, or null)
- **THEN** the request is rejected with 400 naming the `agent` field as
  required, and no session is created

#### Scenario: project id is the wire identifier

- **WHEN** a session is created with `project_id = "proj-abc123def456"` and
  later listed
- **THEN** the session and project payloads reference the project by that
  `project_id`, not by the raw directory path

#### Scenario: agent defaults read and set

- **WHEN** the browser requests `GET /api/agent-defaults` or `PUT
  /api/agent-defaults`
- **THEN** the response is 404 — this endpoint no longer exists. Default
  provider and model are managed through the provider management surface
  (`/api/providers`, `/api/provider-defaults`), and the default agent is
  remembered per project, not through a global agent-defaults endpoint.

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
- **THEN** the listing is rooted at the workspace root (which replaces the
  former work-directory start) and the response `path` echoes its canonical
  form

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
- **THEN** the session is archived, moved from the active session list, and
  the response confirms the action

#### Scenario: archive list endpoint

- **WHEN** `GET /api/archive` is called
- **THEN** the response lists all archived sessions with their original
  project, archive timestamp, and expiry

#### Scenario: browse-dirs rejects a root outside the allowed list

- **WHEN** the workspace root is `/home/op/work`
- **AND** `GET /api/fs/browse-dirs` is called with `root=/etc`
- **THEN** the response is 400 with an out-of-scope error, and no directory
  content outside the workspace root is disclosed

#### Scenario: browse-dirs accepts an allowed root

- **WHEN** a request carries the workspace root itself, or a subpath of it,
  as the explicit `root` (or as `path`)
- **THEN** the listing succeeds for that scope

#### Scenario: unconfigured allowed_roots preserves current behavior

- **WHEN** neither `SEBAS_WORKSPACE_ROOT` nor the config entry provides a
  workspace root
- **THEN** the process working directory becomes the root and a startup
  warning recommends explicit configuration — the former "no constraint when
  unconfigured" behavior is replaced by an always-present boundary

#### Scenario: project registration rejects a path outside allowed roots

- **WHEN** `POST /api/projects` is called with a body path that resolves
  outside the workspace root
- **THEN** the response is 400 with an out-of-scope error and the project is
  not registered

#### Scenario: project registration accepts a path inside allowed roots

- **WHEN** `POST /api/projects` is called with a path inside the workspace
  root
- **THEN** the project registers as before

#### Scenario: project registration without allowed_roots is unchanged

- **WHEN** the config still carries a legacy `[watchdog.webui] allowed_roots`
  key
- **THEN** the key is ignored at parse time and registration is governed
  solely by the workspace root — the whitelist mechanism no longer exists

#### Scenario: out-of-root project is hidden from the project list

- **WHEN** a project registered before the workspace root was tightened
  resolves outside the workspace root
- **AND** the browser requests `GET /api/projects`
- **THEN** that project is not listed

#### Scenario: opening a session of an out-of-root project is rejected

- **WHEN** a session is bound to a local project directory outside the
  workspace root
- **AND** the browser requests the session detail, sends it a message, or
  switches to it
- **THEN** the response is a 4xx out-of-scope rejection, and no spawn or turn
  is started

#### Scenario: housekeeping on an out-of-root session stays available

- **WHEN** a session is bound to a local project directory outside the
  workspace root
- **AND** the browser closes or archives that session
- **THEN** the request succeeds so the out-of-scope session can be cleaned up

#### Scenario: pending submissions ride in the session payload

- **WHEN** the browser requests a session payload while submissions are
  staged during spawn or queued behind a running turn
- **THEN** the payload lists them in delivery order with id, text, position,
  disposition and priority

#### Scenario: pending submission removal and reorder are served

- **WHEN** `POST /api/sessions/{key}/pending/{pending_id}/remove` or
  `.../move` is called for a submission that has not started
- **THEN** the request succeeds and a subsequent session payload reflects the
  new pending list

#### Scenario: managing a started submission is a typed rejection

- **WHEN** either pending endpoint is called for a submission whose turn
  already started
- **THEN** the response is a 4xx typed rejection stating that it is already
  running, and the in-flight turn is unaffected

#### Scenario: queue overflow is a visible rejection

- **WHEN** `POST /api/sessions/{key}/message` is called while the session's
  spawn-window staging queue is at its cap
- **THEN** the response is a 4xx rejection naming the cap, and the response
  does not report the submission as accepted

#### Scenario: cancel forwards to the core channel

- **WHEN** `POST /api/sessions/{key}/cancel` is called for an existing
  session and the core is reachable
- **THEN** the WebUI forwards the cancel request over the core channel and
  returns the outcome

#### Scenario: cancel without the core is honest

- **WHEN** `POST /api/sessions/{key}/cancel` is called while the core is
  unreachable
- **THEN** the response is 503 with the degradation cause, and no success is
  reported

### Requirement: 设置弹窗分区与缺省首项
设置弹窗 SHALL 暴露分区导航，分区 SHALL 按下列顺序排列：`Generic` → `Appearance` →（组间分隔线）`Services` → `Models` → `Agents` →（弹性留白 + 组间分隔线，压底）`Env Vars` · `About`。打开弹窗时缺省聚焦 `Generic` 分区；用户上次停留分区 SHALL 在新会话首次打开时被记住（localStorage），之后打开仍按记忆回到上次分区；记忆中的值若已不存在于分区表（如旧值 `settings`），SHALL 回退到缺省分区。

导航项 SHALL 提供足够的点击目标与选中可见性：行高约 36px、字号不低于 0.875rem、整行 hover 反馈、当前项以左侧 accent 竖条标示。

`Generic` 分区 SHALL 收敛为纯通用可配置项分区，不再承载环境变量表；在语言切换等偏好落地前，主区 SHALL 呈现说明占位文案（指明偏好项后续提供）。

`Agents` 分区 SHALL 承载 agent 目录管理（见 agent-settings 能力）：builtIn `native` 行只读恒在，其余条目可增删改；创建/编辑表单提供 `claude` / `opencode` / 自定义 ACP 三种驱动形态，保存后免重启生效。

`Env Vars` 分区 SHALL 承载环境变量只读表（数据来自 `GET /api/env`，见「环境变量只读展示」），与 `About` 同属底部只读参考组。

`About` 分区 SHALL 分两段呈现：INSTANCE 段在上（工作区根目录 + 复制按钮、当前 default agent kind——读自运行时数据而非写死字面量；不呈现 default provider/model 行——创建预选不再依赖配置默认，见 agent-workbench「Model selector offers the backend catalog before any session」），BUILD 段在下（`/api/about` 的运行时构建信息）。

原 `Settings` 总览分区移除，其维护动作「全部进程重启」与「重置 Settings」SHALL 一并移除——逐服务 restart 由 Services 分区承载，不做广播式入口。

#### Scenario: 缺省聚焦 Generic 分区

- **WHEN** 操作员从侧栏底部打开设置弹窗，且无历史记忆
- **THEN** 弹窗打开后左导航高亮 `Generic`，主区渲染 `Generic` 分区内容（占位文案）

#### Scenario: 历史记忆恢复上次分区

- **WHEN** 操作员上次停留在 `Services` 后关闭弹窗，再打开
- **THEN** 弹窗缺省聚焦 `Services` 分区

#### Scenario: 缺省聚焦 Settings 分区

- **WHEN** localStorage 记忆值为 `settings`（本变更前的合法分区名，`Settings` 总览分区已由 `Generic` 接替）
- **THEN** 左导航高亮 `Generic` 而非报错或空白

#### Scenario: 分区顺序与底部只读组

- **WHEN** 设置弹窗渲染左导航
- **THEN** 分区按 `Generic → Appearance → Services → Models → Agents → Env Vars · About` 排列，`Appearance` 与 `Services` 之间有组间分隔线；`Env Vars` 与 `About` 通过弹性留白压在导航底部、上方有分隔线，与功能区视觉分离

#### Scenario: 分区顺序与 About 压底

- **WHEN** 设置弹窗渲染左导航
- **THEN** 功能分区按 `Generic → Appearance → Services → Models → Agents` 排列，`About` 与 `Env Vars` 同处底部只读组、整体通过弹性留白压底且上方有分隔线

#### Scenario: 聚焦 Agents 分区

- **WHEN** 操作员聚焦 `Agents` 分区
- **THEN** 主区列出 builtIn `native` 行（只读）与全部 agent 条目（可编辑、可删除），并提供新建入口（`claude` / `opencode` / 自定义 ACP 三种驱动形态）

#### Scenario: Settings 分区总览

- **WHEN** 聚焦 `About` 分区
- **THEN** 主区先呈现 INSTANCE 段——工作区根目录（路径 + 复制按钮）、default agent kind（只读，取真实运行时值），后呈现 BUILD 段（版本、commit、构建时间）；INSTANCE 段不含 default provider/model 行

#### Scenario: Generic 分区不再有环境变量表

- **WHEN** 聚焦 `Generic` 分区
- **THEN** 主区呈现偏好占位文案，环境变量只读表不再出现在该分区

#### Scenario: 环境变量表移居 Env Vars 分区

- **WHEN** 聚焦 `Env Vars` 分区
- **THEN** 主区渲染环境变量只读表（名字、解释、按分类展示的值），数据来自 `GET /api/env`

#### Scenario: About 分区承载实例信息

- **WHEN** 聚焦 `About` 分区
- **THEN** 主区先呈现 INSTANCE 段（工作区根目录 + 复制按钮、default agent kind 只读真实值），后呈现 BUILD 段（版本、commit、构建时间）

#### Scenario: About 不再呈现 default provider/model

- **WHEN** 操作员聚焦 `About` 分区，无论当前是否配置了默认 provider/model
- **THEN** INSTANCE 段不渲染 default provider/model 行，也不渲染跳转 Models 分区的对应链接

#### Scenario: 高危动作二次确认

- **WHEN** 设置弹窗任意分区渲染
- **THEN** 不存在「全部进程重启」与「重置 Settings」入口——原高危动作已随 `Settings` 总览分区删除，确认对话框随之消失；逐服务 restart 只在 Services 分区出现
