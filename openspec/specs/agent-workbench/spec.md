## Purpose
Defines the project-centric agent workbench: how a project is registered and
persisted, how sessions are attributed to projects, how the turn stream marks
what arrived while the operator was away, what the composer may promise, and how
several projects' agents are driven concurrently.

## Requirements

### Requirement: Project as the organizing unit

A project SHALL be a directory path on the host, normally a git repository
root. The workbench SHALL present projects as the top-level unit and every
agent session SHALL be reachable through exactly one project grouping.

#### Scenario: register a project

- **WHEN** the operator supplies a directory path that exists on the host
- **THEN** it is registered as a project, appears in the project list, and
  survives a WebUI restart

#### Scenario: path does not exist

- **WHEN** the operator supplies a path that is not an existing directory
- **THEN** registration is rejected with a message naming the path and what is
  wrong with it, and no project is created

#### Scenario: duplicate registration

- **WHEN** the operator registers a path that is already a project
- **THEN** no second project is created and the existing one is surfaced

### Requirement: Project registry persistence is WebUI-owned

The project registry SHALL persist to its own file, separate from the router's
state file. The WebUI SHALL NOT write the router state file, because that file
is rewritten atomically in full by the core on every mutation and concurrent
writers would discard each other's changes.

#### Scenario: registry survives restart

- **WHEN** the WebUI process is restarted after projects were registered
- **THEN** the same projects are listed

#### Scenario: core state untouched

- **WHEN** a project is registered, renamed, or removed
- **THEN** the router state file is not modified

#### Scenario: unreadable registry

- **WHEN** the registry file is absent or cannot be parsed
- **THEN** the workbench starts with an empty project list and reports that the
  registry could not be read, rather than failing to start

### Requirement: Session attribution

A session whose recorded project directory matches a registered project SHALL
group under that project. A session with no recorded project directory —
which is every Feishu-originated session — SHALL group under a distinct
origin-named grouping rather than being hidden or silently attached to a
project.

#### Scenario: workbench-started session attributed

- **WHEN** the operator starts a session from a project
- **THEN** that session records the project's directory and appears under that
  project

#### Scenario: Feishu session grouped by origin

- **WHEN** a session originates in Feishu and has no project directory
- **THEN** it appears under the origin-named grouping, labelled by where it
  came from, and is not attributed to any registered project

#### Scenario: removing a project does not close its sessions

- **WHEN** a project is removed from the registry while it has live sessions
- **THEN** those sessions keep running and remain reachable, and the operator
  is told where they moved

### Requirement: Concurrent projects

The workbench SHALL allow sessions in different projects to be active at the
same time, and switching the displayed project SHALL NOT interrupt, close, or
re-route any other project's session.

#### Scenario: two projects working at once

- **WHEN** a session is working in project A and the operator switches to
  project B and sends a message there
- **THEN** both sessions are working, and project A's session is unaffected

#### Scenario: switching is display-only

- **WHEN** the operator switches the displayed project
- **THEN** no session is closed and no message routing changes

### Requirement: Unseen-turn seam

The turn stream SHALL mark the boundary between turns the operator has already
seen and those that arrived since, showing how many arrived and over what span.
When a session has unseen turns, opening it SHALL position the stream at that
boundary rather than at the newest turn. The seen-boundary SHALL be per-browser
state and SHALL NOT be recorded server-side. The boundary SHALL fall between two turns and SHALL count turns rather than transcript entries, so one agent turn rendered as a single bubble is never split across the seam.

#### Scenario: opening a session with unseen turns

- **WHEN** the operator opens a session that received turns since their last
  visit
- **THEN** the stream opens positioned at the boundary, with the boundary
  marked and the count of turns below it stated

#### Scenario: nothing unseen

- **WHEN** the operator opens a session with no turns since their last visit
- **THEN** no boundary is drawn and the stream opens at the newest turn

#### Scenario: boundary is per-browser

- **WHEN** the operator opens the same session from a different browser
- **THEN** that browser's own seen-boundary applies, and the server holds no
  record of either

#### Scenario: the seam never splits a turn

- **WHEN** the operator opens a session whose unseen turns include one long agent turn composed of streamed text, thinking and tool calls
- **THEN** the boundary is drawn above that whole turn and no part of it appears on the seen side
- **AND** the stated count is the number of turns below the boundary, not the number of transcript entries

### Requirement: Composer promises only what the process can do

The composer SHALL deliver every accepted submission to the core over the session channel, in every process configuration, and SHALL NOT accept a message it cannot deliver. When the channel reports the core unreachable, the composer SHALL render disabled with that cause stated, and SHALL become enabled again on reconnection without a manual reload.

The composer's mode (follow-up vs creation) SHALL be derived from the webui-side focused-session pointer: a focused session puts the composer in follow-up mode targeting that session; no focused session (or an explicit "new session" request) puts it in creation mode. Focusing a session — via the `/api/sessions/{key}/switch` endpoint or by visiting the session's deep-link page — SHALL update the pointer so that a subsequent composer submission follows the focused session rather than spawning a new one.

In creation mode the composer SHALL require an explicit agent choice drawn from `/api/agents`; there is no implicit or "null" agent. The selector SHALL preselect the current project's remembered default agent when one exists.

#### Scenario: composer drives in either configuration

- **WHEN** the workbench runs detached or in-process and the core is reachable
- **THEN** the composer is enabled and a sent message reaches the agent in both

#### Scenario: core unreachable disables with a cause

- **WHEN** the session channel reports the core unreachable
- **THEN** the composer is disabled and states that the core is not connected, rather than presenting an enabled control

#### Scenario: no silent discard

- **WHEN** the core is unreachable
- **THEN** no code path accepts a composer submission and reports success

#### Scenario: recovery needs no reload

- **WHEN** the core returns after being unreachable while the page stays open
- **THEN** the composer becomes enabled again without the operator reloading

#### Scenario: focus follows switch and deep-link

- **WHEN** the operator switches to a session (switch endpoint or deep-link page) and then submits the composer
- **THEN** the submission is delivered to that focused session, not treated as a new-session spawn

#### Scenario: no focus means creation mode

- **WHEN** no session is focused (fresh workbench, or the focused session was just closed)
- **THEN** the composer renders creation mode, and a submission spawns a new session bound to the selected project or inbox

#### Scenario: creation mode requires an explicit agent

- **WHEN** the composer is in creation mode and the operator has not chosen an agent
- **THEN** the submit control is disabled until an agent is chosen from the `/api/agents` list

### Requirement: Session origin is visible

Each session SHALL show whether it originated in Feishu or in the workbench, so
the operator can tell which surface a conversation started on. **Deferred**：
origin 标签尚未在任何视图渲染（会话行携带 `channel` 字段，UI 未消费）——当前
仅能经 Inbox/Projects 分组与键形状间接推断；落一个 origin 徽标属小改动。

#### Scenario: origin shown per session

- **WHEN** a session is displayed
- **THEN** its origin is stated as Feishu or workbench（deferred：当前渲染缺位，
  见上）

### Requirement: Project view states real working-copy context

A project's header SHALL show context the operator recognizes about the
directory — its path and, when the directory is a git repository, its current
branch. When that context cannot be read, the header SHALL omit it rather than
showing a placeholder.

#### Scenario: git project shows branch

- **WHEN** the project directory is a git repository
- **THEN** the header shows the path and current branch

#### Scenario: non-git project omits branch

- **WHEN** the project directory is not a git repository
- **THEN** the header shows the path and no branch field appears

### Requirement: Session execution over the native agent kernel

The system SHALL support routing a session's execution to the native agent kernel (`sebas-agent`) instead of the Claude Code ACP bridge. A session SHALL carry an execution-body hint: `native` (native kernel) or `acp` (ACP bridge, the default). On `native`, the kernel SHALL own the session — session spawning, the turn loop, the tool set (bash / read / write / edit / glob / grep), agent configuration injection (`AGENTS.md` / `CLAUDE.md`), and cancellation/budget semantics. On `acp`, execution proceeds through the ACP child as today.

#### Scenario: WebUI session on the native kernel

- **WHEN** a WebUI create-session request carries `backend = "native"`
- **THEN** the session is created under an `agent-*` session key and its turns/tools run inside the native kernel
- **AND** the session appears in the WebUI snapshot with project_dir honored and zero turns until the first prompt

#### Scenario: Feishu session on the native kernel

- **WHEN** feishu is enabled and a feishu inbound message routes to a `native`-executed session
- **THEN** the session exists in the same shared snapshot visible to the WebUI
- **AND** its tool traces and completion text are readable via the WebUI turn-content API

#### Scenario: Default execution body stays ACP

- **WHEN** no execution-body hint is present (feishu or WebUI default)
- **THEN** the session executes on the ACP bridge as today, preserving behavior

### Requirement: Gated call approval on the native kernel

The native kernel SHALL surface gated tool calls (bash / write / edit / apply_patch in Ask mode) as approval requests rather than executing them. When the WebUI is present, the approval SHALL be presented through the WebUI review-card channel; the operator's decision (allow-once / allow-session / deny with reason) SHALL round-trip to the kernel's approver. Failure to answer SHALL fail closed (the call is not executed).

#### Scenario: WebUI answers a native gated call

- **WHEN** a native session requests approval for a gated tool call
- **THEN** the WebUI review card presents it with decision options
- **AND** an allow-once decision lets only that call through, a deny rejects it, and no answer leaves it unexecuted

#### Scenario: No WebUI attached to a native gated call

- **WHEN** a native session requests approval but no WebUI consumer is attached
- **THEN** the request is denied (fail-closed) and the tool call is not executed

### Requirement: Add project via directory browser

The workbench SHALL provide a modal dialog with a server-side directory browser and a manual path input, either of which SHALL register a project directory. The registered project name SHALL be the directory's basename. The system SHALL probe the directory for a git branch after registration.

#### Scenario: add project via directory browser

- **WHEN** the operator clicks the "Browse Directories…" button in the Add Project dialog
- **THEN** the system fetches a directory listing from `GET /api/fs/browse?path=…`, presents it in a navigable tree, and the operator selects a directory, which registers it as a project

#### Scenario: add project via manual path

- **WHEN** the operator types a path into the manual input field and clicks "Add project"
- **THEN** the path is validated and registered, with the same behaviour as the browser path

#### Scenario: project name from directory name

- **WHEN** a project is registered at `/home/user/work/my-repo`
- **THEN** the project name is `my-repo`

### Requirement: New session without prompt

The workbench SHALL support creating a 0-turn placeholder session without requiring a prompt. The placeholder SHALL appear in the session list immediately and SHALL be activated. An ACP child SHALL NOT be spawned until the first message is sent. Each project row SHALL have a dedicated "New session" button.

#### Scenario: create empty session from project

- **WHEN** the operator clicks the `+` button on a project row
- **THEN** a new session with zero turns is created, the project is selected, the session is activated, and the composer is ready for the first message

#### Scenario: first message spawns the child

- **WHEN** the operator sends a message into a zero-turn placeholder session
- **THEN** the system spawns the ACP child and the session transitions to working

### Requirement: Session archive

Each session row SHALL have an archive button that moves the session to the History group. An archived session SHALL be read-only — the operator cannot send messages into it, cannot close it, and cannot switch to it as the active session. An archived session SHALL be restorable to its original project by clicking it in the History group.

#### Scenario: archive a session

- **WHEN** the operator clicks the archive button on a session row
- **THEN** the session is moved to the History group, marked as read-only, and the operator cannot interact with it

#### Scenario: restore archived session

- **WHEN** the operator clicks an archived session in the History group
- **THEN** the session is restored to its original project, becomes writable, and is activated

### Requirement: History group is the archive

The History group SHALL contain only archived sessions. Sessions with no project directory (Feishu-originated sessions) SHALL appear in a separate Inbox group instead. The History group SHALL show the total count of archived sessions and be collapsible.

#### Scenario: History holds only archived sessions

- **WHEN** the History group is expanded
- **THEN** every session listed has been archived and none are Feishu-originated sessions without a project

#### Scenario: Inbox for unbound sessions

- **WHEN** a session has no project directory
- **THEN** it appears in the Inbox group, not in History

### Requirement: Archive expiry

The system SHALL permanently delete archived sessions whose `archived_at` timestamp is older than the configured retention period. The default retention SHALL be 30 days, configurable via `[webui] archive_retention_days` in the config file. Expired sessions SHALL be removed on WebUI startup and on every session list request.

#### Scenario: expired session cleaned up

- **WHEN** the WebUI starts or the session list is requested and an archived session exceeds the retention period
- **THEN** that session is permanently removed from the archive list

#### Scenario: retention configured

- **WHEN** `[webui] archive_retention_days = 7` is set in the config
- **THEN** archived sessions older than 7 days are removed at startup and on list requests

### Requirement: Execution-body availability is stated, not discovered

The composer's execution-body selector SHALL reflect, for each execution body,
whether it can serve new sessions in the current process configuration. An
execution body that cannot serve new sessions — for example the native kernel
running without provider credentials — SHALL be presented as unavailable with
its cause stated, and SHALL NOT be selectable such that the operator only
discovers the failure on submission. Availability SHALL be derived from the
session backend's own report of both execution bodies, not from the ACP side
alone.

#### Scenario: native kernel without credentials shown as unavailable

- **WHEN** the native kernel has no provider credentials and the composer is
  rendered
- **THEN** the `native` option is shown as unavailable with the cause stated,
  and submitting a native spawn is prevented at the composer rather than
  failing at the core

#### Scenario: both bodies available

- **WHEN** both the ACP bridge and the native kernel can serve new sessions
- **THEN** the selector offers both without degradation notices

#### Scenario: availability recovers without reload

- **WHEN** the cause making an execution body unavailable is resolved while the
  page stays open
- **THEN** the selector offers that body again without the operator reloading

### Requirement: Model selection covers the native kernel

The composer SHALL offer model selection for native-kernel sessions drawn from
the models the native execution body exposes, and a model chosen for a native
session SHALL apply to that session's subsequent turns. A native session with
no model selected SHALL use the kernel's default. Sessions on other execution
bodies keep their existing model-selection behavior unchanged.

#### Scenario: native session model dropdown is populated

- **WHEN** the operator opens the model dropdown for a native-kernel session
- **THEN** it lists the models the native execution body exposes rather than
  being empty

#### Scenario: chosen model applies to subsequent turns

- **WHEN** the operator selects a model on a native session and sends a message
- **THEN** the turn runs with the selected model, and the session's current
  model is visible afterwards

#### Scenario: unselected falls back to default

- **WHEN** a native session is created with no model chosen
- **THEN** its turns run with the native kernel's default model and the
  composer shows no false "selected" state

### Requirement: Model selector offers the backend catalog before any session

The composer's model selector SHALL offer the catalog the operator configured in
Settings — every configured provider's model list, presented as two levels
(provider, then model) — before any session exists, so the operator can pick a
model for the first turn of a new session. The default selection SHALL follow
the configured default provider and model. When a session exists, the selector SHALL offer that session's
`available_models` instead, because a mid-session switch is valid only if the
session's execution body accepts the chosen model.
When neither a configured catalog nor a session model list is available, the
selector SHALL state its unavailability honestly rather than offering an empty
or fabricated list. Changes to the configured catalog or the default SHALL be
reflected without requiring a session to be created first. The selector SHALL
NOT derive its options from another session's `available_models`.

#### Scenario: selector populated before any session

- **WHEN** a default provider with a models catalog is configured and the
  operator opens a fresh workbench with no sessions
- **THEN** the model selector offers the catalog's models

#### Scenario: provider and model are chosen in two levels

- **WHEN** the operator opens a fresh workbench with two providers configured in Settings
- **THEN** the selector first offers the providers, and choosing one offers that provider's models

#### Scenario: existing sessions keep session-sourced options

- **WHEN** a session exposes `available_models` and the operator opens its
  model dropdown
- **THEN** the selector offers that session's options, matching the current
  behavior

#### Scenario: no catalog and no session models is stated honestly

- **WHEN** no default provider catalog exists and no session exposes models
- **THEN** the selector presents an explicit unavailability indication rather
  than an empty list

### Requirement: Rail project removal entry

Each project row in the workbench rail SHALL expose a remove action (hover-revealed, consistent with the existing row-action affordance). Triggering it SHALL open a confirmation dialog that names the project and states that live sessions under it keep running and migrate to the Inbox. Confirming SHALL call `POST /api/projects/{id}/remove` and remove the row from the rail without a page reload; the dialog SHALL present the typed error inline when the backend rejects the removal. Cancelling SHALL leave the registry untouched.

#### Scenario: remove a project from the rail

- **WHEN** the operator clicks the remove button on a project row and confirms the dialog
- **THEN** `POST /api/projects/{id}/remove` is called, the project disappears from the rail, and no page reload is required

#### Scenario: live sessions survive project removal

- **WHEN** a project with live sessions is removed from the rail
- **THEN** the sessions keep running and appear under the Inbox group, and the confirmation dialog has already told the operator this would happen

#### Scenario: removal rejection surfaces inline

- **WHEN** the backend rejects the removal (unknown path, core-side error)
- **THEN** the dialog presents the backend's typed error inline and the project row stays in the rail

#### Scenario: cancel leaves the registry untouched

- **WHEN** the operator opens the remove confirmation and clicks cancel
- **THEN** no remove request is sent and the project stays registered

### Requirement: Rail session close entry

Each session row in the workbench rail (project groups and Inbox) SHALL expose a close action alongside the existing archive button, with the same semantics as `POST /api/sessions/{key}/close` (kill the child when active, drop the mapping). Closing SHALL require no confirmation for inactive (dormant/done/failed) sessions and SHALL require an inline confirmation for active (starting/queued/working) sessions. When the session being closed has pending submissions, the confirmation SHALL state how many will be discarded and never executed. When the closed session was the focused one, the workbench SHALL return to the no-focus empty state.

#### Scenario: close a dormant session from the rail

- **WHEN** the operator clicks the close button on a dormant session row
- **THEN** the session is closed immediately, the row disappears, and no confirmation dialog is shown

#### Scenario: closing a working session asks first

- **WHEN** the operator clicks the close button on a session whose child is still running
- **THEN** an inline confirmation is shown first, and only on confirm is the close request sent

#### Scenario: closing with pending submissions names the loss

- **WHEN** the operator clicks the close button on a session that has pending submissions
- **THEN** the confirmation states how many pending submissions will be discarded, and closing removes them without delivering them to any later session

#### Scenario: closing the focused session clears the stage

- **WHEN** the operator closes the session that is currently focused
- **THEN** the workbench stage returns to the no-focus empty state and the composer re-enters creation mode

### Requirement: Placeholder session is immediately writable

After the operator creates a 0-turn placeholder session (rail `+` button), the workbench composer SHALL enter follow-up mode for that session — the focused-session pointer SHALL be set so that the first composer submission is delivered to the placeholder via `POST /api/sessions/{key}/message` and SHALL NOT be interpreted as a request to spawn another session.

#### Scenario: first message reaches the placeholder session

- **WHEN** the operator creates a placeholder session from the rail and types a message in the workbench composer
- **THEN** the message is posted to the placeholder session's message endpoint, the session spawns its child, and no second session is created

#### Scenario: composer shows follow-up mode after placeholder creation

- **WHEN** a placeholder session has just been created and focused
- **THEN** the composer renders its follow-up mode (read-only agent label, no execution-backend dropdown) for that session

### Requirement: Session agent binding is immutable

A session SHALL record the agent it was created with, and that binding SHALL NOT change for the lifetime of the session. Any API or channel request that attempts to re-target an existing session to a different agent SHALL be rejected with a typed rejection. Models within the bound agent MAY be changed mid-session via the existing model endpoint; agents themselves are not interchangeable because different agents cannot resume each other's conversation state.

#### Scenario: agent is locked at creation

- **WHEN** a session is created with `agent = "claudecode"`
- **THEN** every subsequent turn for that session runs on `claudecode`, and no request can re-target it to another agent

#### Scenario: attempt to change agent is rejected

- **WHEN** a client sends a request that would change an existing session's agent
- **THEN** the backend rejects it with a typed rejection (4xx) and the session continues on its original agent

#### Scenario: model may change, agent may not

- **WHEN** the operator changes the model on a session bound to `claudecode`
- **THEN** the model change applies to subsequent turns, and the session's agent remains `claudecode`

#### Scenario: UI communicates immutability

- **WHEN** a session is displayed in the workbench or detail view
- **THEN** the bound agent is shown as read-only with a lock affordance and tooltip stating it was chosen at creation, and no agent-switch control is rendered

### Requirement: Composer submissions always deliver

A composer submission that the UI reports as accepted SHALL be delivered to the agent — the system SHALL NOT acknowledge a message and then leave the session without a spawned child. A placeholder session awaiting its first prompt SHALL be identified by an explicit marker, not inferred from optional fields; the first message on such a session SHALL trigger spawn and SHALL NOT be enqueued without a consumer.

#### Scenario: first message on a placeholder spawns the child

- **WHEN** a placeholder session created with any valid `agent` value receives its first composer message
- **THEN** the message spawns the agent child and the session transitions to working, regardless of which agent id was chosen

#### Scenario: accepted means delivered

- **WHEN** `POST /api/sessions/{key}/message` returns 200
- **THEN** the message is either already delivered to a live child, queued on a live spawn that will drain it, or the response was not 200; no code path acknowledges a message that will never be processed

#### Scenario: placeholder marker survives restart

- **WHEN** the daemon restarts after a placeholder session was created
- **THEN** the restored mapping still identifies the session as awaiting its first prompt, and the first post-restart message spawns the child

### Requirement: Project-level default agent

Each project SHALL remember the agent most recently used to create a session under it. When the operator focuses that project and the composer is in creation mode, the agent selector SHALL preselect that remembered agent. The default is stored in the project registry and survives a restart.

#### Scenario: default agent follows last use

- **WHEN** the operator creates a session in project A with `agent = "codex"` and later returns to project A in creation mode
- **THEN** the agent selector preselects `codex`

#### Scenario: different projects remember different agents

- **WHEN** project A was last used with `codex` and project B with `claudecode`
- **THEN** switching to project A preselects `codex` and switching to project B preselects `claudecode`

#### Scenario: first visit falls back honestly

- **WHEN** a project has no recorded default agent
- **THEN** the selector preselects the first reachable agent and marks no project default as chosen

### Requirement: Pending submissions stack above the composer

The workbench SHALL render the focused session's pending submissions directly above the composer input, in delivery order, without occupying transcript space. Each entry SHALL state its disposition: submissions staged during a spawn SHALL read as combining into the session's first message, and submissions queued behind a running turn SHALL read as waiting, with their position. A submission SHALL NOT appear in the transcript until it starts (or is combined at activation); at that moment it SHALL leave the stack and the transcript SHALL show it as a submission entry.

The stack SHALL support removal of any entry and drag-reordering within the entry's own disposition group. Entries carrying priority (`/btw`) SHALL be rendered as priority and SHALL NOT be draggable, and no drag SHALL place a non-priority entry ahead of a priority one. Submitting while entries are pending SHALL append to the stack — it SHALL NOT replace, discard, or silently merge into an existing entry's text.

When the session ends or is closed while entries are pending, those entries SHALL be reported as not executed in a single explicit notice naming them (the stack itself disappears with the session, so the notice is the record); the stack SHALL never shrink without an explanation.

#### Scenario: stack renders pending submissions in order

- **WHEN** the focused session has submissions staged during spawn and submissions queued behind a running turn
- **THEN** the region above the composer lists them in delivery order, distinguishing "combining into the first message" from "waiting" with position

#### Scenario: submitting appends instead of replacing

- **WHEN** the operator submits a message while entries are already pending
- **THEN** the new submission appears as an additional stack entry and no existing entry's text is altered

#### Scenario: remove and reorder take effect

- **WHEN** the operator removes a pending entry, or drags a non-priority entry to a new position in its group
- **THEN** the stack reflects the change immediately and still reflects it after the next refresh

#### Scenario: priority entries are pinned

- **WHEN** the stack contains a `/btw` priority entry
- **THEN** it is rendered as priority, cannot be dragged, and no drag can place another entry ahead of it

#### Scenario: a started submission leaves the stack and enters the transcript

- **WHEN** a pending submission starts its turn (or is combined at activation)
- **THEN** it is removed from the stack and appears in the transcript as a submission entry, so the conversation shows what is actually running

#### Scenario: pending entries are not silently dropped at session end

- **WHEN** the focused session fails or is closed while entries are pending
- **THEN** a notice names those entries as not executed, and the stack clears only together with that notice

### Requirement: Workbench renders the focused session as a conversation

The workbench SHALL render the focused session as a conversation between the
operator and the agent, in transcript order: each submission the operator made
SHALL appear as their own turn, and each agent turn SHALL appear as a single
assistant bubble. One agent turn SHALL be composed of everything the agent
produced for that turn — the streamed text concatenated in arrival order, its
thinking, and its tool invocations — and SHALL NOT be rendered as a series of
per-chunk bubbles. Thinking SHALL be folded inside the turn's bubble, and tool
invocations SHALL be grouped inside the bubble as an expandable "used N tools"
group rather than presented as ordinary prose. A submission SHALL appear in the
conversation only when its turn starts.

#### Scenario: both sides of the conversation are visible

- **WHEN** the operator opens a session in which they submitted messages across several turns
- **THEN** the workbench shows their submissions and the agent's replies in transcript order, each submission as the operator's own turn

#### Scenario: one agent turn is one bubble

- **WHEN** an agent turn arrives as many streamed text chunks plus thinking plus tool invocations
- **THEN** the workbench renders one assistant bubble for that turn, with the text in order, thinking folded inside, and the tool invocations collected in one expandable group

#### Scenario: tool invocations are not mistaken for prose

- **WHEN** an agent turn invokes tools
- **THEN** those invocations are rendered as the turn's tool group and are distinguishable from the turn's prose

#### Scenario: a submission appears when its turn starts

- **WHEN** a submission is accepted while the agent is still working
- **THEN** it is not rendered as a started turn until its turn actually begins

### Requirement: Workbench is the single conversation surface

The workbench SHALL be the only conversation surface. Selecting a session in the
rail SHALL focus it in place — the operator SHALL NOT be navigated away from the
workbench to a separate detail page. The `/sessions/{key}` deep link SHALL keep
resolving and SHALL render the same workbench with that session focused, so
bookmarks and links keep working. The rail's current-session marker SHALL follow
the focused-session pointer rather than the browser location. Every per-session
action the retired detail page offered — close, archive, and the gated-call
review cards — SHALL remain reachable from the workbench.

#### Scenario: selecting a session keeps the operator in the workbench

- **WHEN** the operator selects a session in the rail
- **THEN** that session becomes the focused one and the workbench renders its conversation without a page change to a different surface

#### Scenario: deep link renders the workbench

- **WHEN** a bookmarked `/sessions/{key}` is opened
- **THEN** the workbench renders with that session focused, rather than a separate detail page

#### Scenario: the rail marker follows focus

- **WHEN** the focused session changes through any supported path
- **THEN** the rail marks the focused session as current regardless of the browser location

#### Scenario: per-session actions stay reachable

- **WHEN** the operator focuses a session whose child is running
- **THEN** close, archive and that session's gated-call review cards are reachable from the workbench
