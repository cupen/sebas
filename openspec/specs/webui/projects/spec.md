# webui/projects Specification

## Purpose
Lets users open a git repository as a project in the WebUI, spawn a Claude
Code agent in that directory, and interact with it through the browser —
turning the WebUI from a session viewer into a project-oriented agent
workspace.

## Requirements

### Requirement: Agent project page

The WebUI SHALL serve `GET /agent` as a project-oriented agent workspace page
with a sidebar listing all sessions (tagged by project directory or chat source)
and a main area for the focused session's chat timeline and composer. The sidebar
SHALL include a "+ New Project" button that reveals an inline form for entering
a git repository path.

#### Scenario: agent page renders sidebar and empty state

- **WHEN** a browser requests `GET /agent`
- **THEN** the response renders the agent workspace layout with a sidebar
  listing all sessions (each showing project_dir or chat_id, status badge, and
  last-active time), and the main area showing the empty state prompt
  "Open a project to start working with Claude Code"

#### Scenario: sidebar active highlight

- **WHEN** a session is focused and the agent page is loaded
- **THEN** the corresponding sidebar item SHALL have an `active` CSS class

### Requirement: Create project session

The WebUI SHALL accept a git repository path via `POST /api/agent/projects`
with form field `path`, expand `~` and resolve it to an absolute path, and
call `web_spawn(prompt, project_dir)` to create a new session. The prompt
SHALL be auto-generated as "Work in {absolute_path} — understand the project
structure and help the user with their tasks." The endpoint SHALL return the
encoded session key as JSON `{ "key": "..." }`.

#### Scenario: create project from path

- **WHEN** a user posts `path=~/projects/my-app` to `POST /api/agent/projects`
- **THEN** the system expands `~` to the home directory, resolves the absolute
  path, spawns a new agent session with `project_dir` set to that path, and
  returns `{ "key": "<encoded_key>" }` with status 201

#### Scenario: path not found

- **WHEN** the posted `path` does not exist or is not a directory
- **THEN** the endpoint returns 400 with an error message

### Requirement: Agent session message

The WebUI SHALL accept messages via `POST /api/agent/{key}/message` with form
field `message` and forward them to the existing session via `web_send_message`.

#### Scenario: send message to agent session

- **WHEN** a user posts `message=explain the main.rs` to
  `POST /api/agent/{key}/message`
- **THEN** the message is routed to the session and the timeline is re-rendered

### Requirement: Agent session detail and timeline

The WebUI SHALL serve `GET /agent/{key}` for the full agent session detail
page (chat timeline + composer) and `GET /agent/{key}/timeline` for the
timeline fragment (HTMX poll target).

#### Scenario: agent detail page

- **WHEN** a browser requests `GET /agent/{key}`
- **THEN** the response renders the agent workspace layout with the focused
  session's chat timeline and composer, and the sidebar with the active session
  highlighted

#### Scenario: timeline poll

- **WHEN** a browser requests `GET /agent/{key}/timeline`
- **THEN** the response is a partial HTML fragment of the session's card body
  elements, suitable for HTMX `hx-swap="innerHTML"`

### Requirement: Session model with project metadata

The `SessionRow` model SHALL carry `project_dir: Option<String>`,
`prompt_preview: Option<String>`, and `agent_kind: Option<String>` fields.
`agent_kind` SHALL be the execution-backend kind recorded when the session was
created (`None` = the configured default kind), and SHALL survive core
restarts through session persistence. The agent page template SHALL display
`project_dir` (with a 📁 icon) when present, and `prompt_preview` (with a 💬
icon) otherwise. The session detail payload and the summary's focused-session
entry SHALL include the same `agent_kind` value, and the session detail header
SHALL render the agent kind as read-only text.

#### Scenario: session row displays project dir

- **WHEN** a session has `project_dir` set to `/home/user/projects/my-app`
- **THEN** the sidebar item SHALL show `📁 /home/user/projects/my-app` with
  a `codex-dir-icon` class

#### Scenario: session row without project

- **WHEN** a session has no `project_dir` (Feishu-originated or legacy)
- **THEN** the sidebar item SHALL show the `prompt_preview` or `chat_id` with
  a 💬 icon

#### Scenario: agent kind exposed and rendered

- **WHEN** a session was created with backend hint `acp:claude`
- **THEN** the session row, session detail, and summary focused-session entry
  report `agent_kind` as `claude`
- **AND** the session detail header and the follow-up composer render the
  agent kind as read-only text

#### Scenario: agent kind falls back for default and legacy sessions

- **WHEN** a session was created without an explicit agent kind, or its
  persisted record predates the field
- **THEN** `agent_kind` is `None` and the UI renders the default-kind label
  instead of an error

### Requirement: Navigation tab

The WebUI's navigation sidebar SHALL include an "Agent" tab linking to
`/agent`, alongside the existing "Dashboard" and "Sessions" tabs.

#### Scenario: agent tab is visible

- **WHEN** a user browses any page of the WebUI
- **THEN** the sidebar SHALL contain a link to `/agent` labeled "Agent"

### Requirement: Conversation-area composer is session-scoped

The workbench conversation area's composer SHALL operate in one of two modes.
When a session is focused, the composer SHALL be in follow-up mode: submitting
a message SHALL send it to the focused session, and the composer SHALL NOT
offer any agent (execution-backend) selection — the focused session's agent
SHALL be displayed as read-only, small, non-interactive text in the composer's
bottom toolbar, next to a model dropdown sourced from that session's
selectable models. When no session is focused, or when the operator explicitly
switches the composer via its visible "new session" affordance, the composer
SHALL be in creation mode: submitting SHALL create a new session, with the
agent dropdown (one entry per reachable agent plus `native`, per the
agent-driver requirement) and the model dropdown both in the bottom toolbar,
and the target binding (project or inbox) displayed. The composer SHALL NOT
render an agent control outside creation mode.

#### Scenario: follow-up send routes to the focused session

- **WHEN** a session is focused and the operator submits a message from the
  conversation-area composer
- **THEN** the message is delivered to the focused session (no new session is
  created)

#### Scenario: agent is read-only in follow-up mode

- **WHEN** a session is focused and its agent kind is known
- **THEN** the composer's bottom toolbar shows that agent kind as small
  read-only text with no dropdown or button semantics
- **AND** the model dropdown next to it lists that session's selectable
  models with its current model preselected

#### Scenario: explicit switch to creation while a session is focused

- **WHEN** the operator activates the composer's "new session" affordance
- **THEN** the composer switches to creation mode, exposing the agent dropdown
  and binding display
- **AND** submitting creates a new session without disturbing the previously
  focused session's transcript

#### Scenario: no agent control outside creation mode

- **WHEN** the composer is in follow-up mode
- **THEN** no element in the composer changes the session's execution backend
