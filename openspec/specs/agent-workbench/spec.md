## Purpose
Defines the project-centric agent workbench: how a project is registered and
persisted, how sessions are attributed to projects, how the turn stream marks
what arrived while the operator was away, what the composer may promise, and how
several projects' agents are driven concurrently.

## Requirements



### Requirement: Project as the organizing unit

A project SHALL be a directory path on a named execution node, normally a git
repository root; the host running the workbench is itself one such node. The
workbench SHALL present projects as the top-level unit, SHALL name the node each
project lives on, and every agent session SHALL be reachable through exactly one
project grouping. Whether a path is usable SHALL be judged by the node the
project names, not by the workbench host.

#### Scenario: register a project

- **WHEN** the operator supplies a directory path for a named node and the path
  is an existing directory on that node
- **THEN** it is registered as a project, appears in the project list naming its
  node, and survives a WebUI restart

#### Scenario: path does not exist

- **WHEN** the operator supplies a path that is not an existing directory on the
  named node
- **THEN** registration is rejected with a message naming the node, the path and
  what is wrong with it, and no project is created

#### Scenario: duplicate registration

- **WHEN** the operator registers a path that is already a project on the same
  node
- **THEN** no second project is created and the existing one is surfaced

#### Scenario: the same path on two nodes is two projects

- **WHEN** the operator registers the same path against two different nodes
- **THEN** two distinct project entries exist, each naming its node, and neither
  is treated as the other

#### Scenario: local registration stays implicit

- **WHEN** the operator registers a path without naming a node
- **THEN** it is registered against the local node, preserving the behavior of
  existing registrations

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

A session SHALL record the node and project directory it runs in, and SHALL
group under the project entry that matches both. A session with no recorded
project directory — which is every Feishu-originated session — SHALL group under
a distinct origin-named grouping rather than being hidden or silently attached
to a project, and SHALL be placed on the configured default execution node.

#### Scenario: workbench-started session attributed

- **WHEN** the operator starts a session from a project
- **THEN** that session records the project's node and directory and appears
  under that project

#### Scenario: Feishu session grouped by origin

- **WHEN** a session originates in Feishu and has no project directory
- **THEN** it appears under the origin-named grouping, labelled by where it
  came from, and is not attributed to any registered project
- **AND** it runs on the default execution node, which its grouping names

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

The composer SHALL always be in follow-up mode targeting the webui-side focused-session pointer; it SHALL NOT offer session creation — creation lives in the rail's creation dialog. Focusing a session — via the `/api/sessions/{key}/switch` endpoint, the rail creation dialog, or by visiting the session's deep-link page — SHALL update the pointer so that a composer submission follows the focused session. When no session is focused, the workbench SHALL present the no-focus empty state, the composer SHALL render no creation controls, and an explicit hint SHALL direct the operator to the rail's creation entry.

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
- **THEN** the submission is delivered to that focused session

#### Scenario: no focus means creation mode

- **WHEN** no session is focused (fresh workbench, or the focused session was just closed)
- **THEN** the workbench presents the no-focus empty state, the composer renders no creation controls and no agent selector, and an explicit hint directs the operator to the rail's creation entry instead of offering in-composer creation

#### Scenario: creation mode requires an explicit agent

- **WHEN** the rail creation dialog is open and the operator has not chosen an agent
- **THEN** the dialog's confirm control is disabled until an agent is chosen from the `/api/agents` list — the composer itself renders no agent choice

#### Scenario: no creation chip in the composer

- **WHEN** a session is focused and the operator looks at the composer toolbar
- **THEN** no "new session" control is rendered — a new session can only be started from the rail's creation dialog

### Requirement: Session origin is visible

Each session SHALL show whether it originated in Feishu or in the workbench, so
the operator can tell which surface a conversation started on. **Deferred**：
origin 标签尚未在任何视图渲染（会话行携带 `channel` 字段，UI 未消费）——当前
仅能经会话键形状间接推断；落一个 origin 徽标属小改动。

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

The workbench SHALL support creating a 0-turn placeholder session without requiring a prompt. The placeholder SHALL appear in the session list immediately and SHALL be activated. An ACP child SHALL NOT be spawned until the first message is sent. Each project row SHALL have a dedicated "New session" button (the rail carries no Inbox group — sessions with no project are not created from the workbench). Clicking it SHALL open a creation dialog — the ONLY place an agent can be chosen — which SHALL require an explicit agent choice drawn from `/api/agents` (no implicit or "null" agent), SHALL preselect the target project's remembered default agent when one exists, SHALL carry an optional permission-mode choice (`ask | edit | allow | auto`, defaulting to "agent default" which omits the `mode` field on the wire), and MAY carry an optional two-level model choice (provider, then model) drawn from the Settings catalog, preselected per the configured default provider and model. Confirming the dialog SHALL create and activate the placeholder; cancelling SHALL create nothing.

#### Scenario: dialog requires an explicit agent

- **WHEN** the operator opens the creation dialog and has not chosen an agent
- **THEN** the confirm control is disabled until an agent is chosen from the `/api/agents` list

#### Scenario: create empty session from project

- **WHEN** the operator confirms the creation dialog opened from a project row's `+` button
- **THEN** a new session with zero turns is created bound to that project and the chosen agent, the project is selected, the session is activated, and the composer is ready for the first message

#### Scenario: cancel creates nothing

- **WHEN** the operator cancels the creation dialog
- **THEN** no session is created and no focus change occurs

#### Scenario: first message spawns the child

- **WHEN** the operator sends a message into a zero-turn placeholder session
- **THEN** the system spawns the ACP child and the session transitions to working

#### Scenario: dialog carries the permission-mode choice

- **WHEN** the creation dialog is open and the operator leaves the mode dropdown on its default
- **THEN** confirming creates the session without a `mode` field on the wire; choosing `edit` creates it with `mode=edit`

### Requirement: Session archive

Each session row SHALL have an archive button that moves the session to the History group. An archived session SHALL be read-only — the operator cannot send messages into it, cannot close it, and cannot switch to it as the active session. An archived session SHALL be restorable to its original project by clicking it in the History group.

#### Scenario: archive a session

- **WHEN** the operator clicks the archive button on a session row
- **THEN** the session is moved to the History group, marked as read-only, and the operator cannot interact with it

#### Scenario: restore archived session

- **WHEN** the operator clicks an archived session in the History group
- **THEN** the session is restored to its original project, becomes writable, and is activated

### Requirement: History group is the archive

The History group SHALL contain only archived sessions, listed newest-first by archive time. The rail SHALL NOT render an Inbox group: sessions with no project directory SHALL NOT be listed in the rail (they remain accessible through the sessions API and their originating surface). The History group SHALL show the total count of archived sessions and be collapsible.

#### Scenario: History holds only archived sessions

- **WHEN** the History group is expanded
- **THEN** every session listed has been archived

#### Scenario: History is sorted newest-first

- **WHEN** sessions are archived at different times
- **THEN** the History group lists them in descending order of archive time

#### Scenario: Inbox for unbound sessions

- **WHEN** a session has no project directory
- **THEN** it appears in no rail group — the Inbox group no longer exists — and History does not list it either

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

The creation dialog's model selector SHALL offer the catalog the operator configured in Settings — every configured provider's model list, presented as two levels (provider, then model) — so the operator can pick a model for the first turn of a new session. The default selection SHALL follow the configured default provider and model. When the catalog is empty or unavailable, the dialog SHALL state its unavailability honestly rather than offering an empty or fabricated list. Changes to the configured catalog or the default SHALL be reflected in the dialog without requiring a restart.

The workbench composer SHALL present the focused session's model selection as a single chip at the composer's bottom-right, offering that session's `available_models`, because a mid-session switch is valid only if the session's execution body accepts the chosen model. The chip SHALL NOT derive its options from the catalog or from another session's `available_models`. When the focused session exposes no models, the chip SHALL state that honestly rather than rendering an empty menu.

#### Scenario: selector populated before any session

- **WHEN** a default provider with a models catalog is configured and the operator opens the creation dialog
- **THEN** the dialog's model selector offers the catalog's models

#### Scenario: provider and model are chosen in two levels

- **WHEN** the creation dialog is open with two providers configured in Settings
- **THEN** the selector first offers the providers, and choosing one offers that provider's models

#### Scenario: existing sessions keep session-sourced options

- **WHEN** a session exposes `available_models` and the operator opens the composer's model chip
- **THEN** the chip offers that session's options in a two-level menu grouped by provider, with the current model marked

#### Scenario: no catalog and no session models is stated honestly

- **WHEN** no provider catalog exists and the focused session exposes no models
- **THEN** the creation dialog and the composer chip each present an explicit unavailability indication rather than an empty list

#### Scenario: chip without session models is stated honestly

- **WHEN** the focused session exposes no `available_models`
- **THEN** the chip presents an explicit unavailability indication rather than an empty menu

### Requirement: Rail project removal entry

Each project row SHALL expose its remove action inside the row's overflow (`...`) menu. When the project has at least one non-archived session, the removal SHALL be rejected — the backend SHALL refuse the remove request and the dialog SHALL state that sessions must be archived or closed first, naming the session count. When no non-archived sessions remain, confirming the dialog SHALL call `POST /api/projects/{id}/remove` and the project row SHALL disappear from the rail without a page reload; the dialog SHALL present the typed error inline when the backend rejects the removal. Cancelling SHALL leave the registry untouched.

#### Scenario: remove a project from the rail

- **WHEN** the operator removes a project that has no non-archived sessions and confirms the dialog
- **THEN** `POST /api/projects/{id}/remove` is called, the project disappears from the rail, and no page reload is required

#### Scenario: live sessions survive project removal

- **WHEN** a project with live sessions is about to be removed from the rail
- **THEN** the removal is refused with a message naming the session count, so the sessions stay registered under their project until they are archived or closed

#### Scenario: removal rejection surfaces inline

- **WHEN** the backend rejects the removal (core-side error)
- **THEN** the dialog presents the backend's typed error inline and the project row stays in the rail

#### Scenario: cancel leaves the registry untouched

- **WHEN** the operator opens the remove confirmation and clicks cancel
- **THEN** no remove request is sent and the project stays registered

### Requirement: Rail session close entry

Each session row in the workbench rail SHALL expose a close action inside the row's overflow (`...`) menu, with the same semantics as `POST /api/sessions/{key}/close` (kill the child when active, drop the mapping). Closing SHALL require no confirmation for inactive (dormant/done/failed) sessions and SHALL require an inline confirmation for active (starting/queued/working) sessions. When the session being closed has pending submissions, the confirmation SHALL state how many will be discarded and never executed. When the closed session was the focused one, the workbench SHALL return to the no-focus empty state.

#### Scenario: close a dormant session from the rail

- **WHEN** the operator picks Close from a dormant session row's overflow menu
- **THEN** the session is closed immediately, the row disappears, and no confirmation dialog is shown

#### Scenario: closing a working session asks first

- **WHEN** the operator picks Close on a session whose child is still running
- **THEN** an inline confirmation is shown first, and only on confirm is the close request sent

#### Scenario: closing with pending submissions names the loss

- **WHEN** the operator picks Close on a session that has pending submissions
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

Each project SHALL remember the agent most recently used to create a session under it. When the operator opens the creation dialog for that project, the agent selector SHALL preselect that remembered agent. The default is stored in the project registry and survives a restart.

#### Scenario: default agent follows last use

- **WHEN** the operator creates a session in project A with `agent = "codex"` and later opens project A's creation dialog
- **THEN** the agent selector preselects `codex`

#### Scenario: different projects remember different agents

- **WHEN** project A was last used with `codex` and project B with `claudecode`
- **THEN** project A's dialog preselects `codex` and project B's dialog preselects `claudecode`

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

### Requirement: Execution node availability is stated, not discovered

The workbench SHALL show, for each project and session, the node it belongs to
and whether that node is currently online. A node that is offline SHALL be
presented as offline with its cause stated, and the composer SHALL prevent
starting a session against a project whose node is offline rather than failing
on submission. Availability SHALL recover without the operator reloading the
page.

#### Scenario: offline node is visible before submission

- **WHEN** a project's node is offline and the operator opens that project
- **THEN** the project is marked with its node offline and the cause, and
  starting a session from it is prevented at the composer

#### Scenario: availability recovers without reload

- **WHEN** the node comes back online while the page stays open
- **THEN** the project is offered again and starting a session becomes possible
  without a reload

### Requirement: Desired and effective session mode are both visible

The workbench SHALL show a remote session's desired mode alongside the mode
actually enforced by its execution body, and SHALL state plainly when the two
differ because the execution body cannot enforce the desired mode. A session
whose mode is `auto` SHALL be visually distinguishable from a gated session.

#### Scenario: unenforceable mode is shown as such

- **WHEN** a session's execution body cannot enforce the desired mode
- **THEN** the session shows both values and states that the desired mode is not
  enforced

#### Scenario: auto session is distinguishable

- **WHEN** a session runs in `auto` mode
- **THEN** the workbench marks it as ungated so an operator can tell it apart
  from a session that asks for decisions

### Requirement: Parked remote approvals surface in the workbench

Permission requests that stayed parked while the control plane was away SHALL be
surfaced to the operator on return, grouped so that a session waiting on a
decision is distinguishable from one that is working. A session waiting on a
parked decision SHALL be presented as waiting, not as running.

#### Scenario: returning operator sees what is waiting

- **WHEN** the operator returns to a control plane that was away while requests
  were parked
- **THEN** every session waiting on a decision is presented as waiting, with its
  parked requests reachable

#### Scenario: waiting is not reported as working

- **WHEN** a remote session is blocked on an unanswered permission request
- **THEN** the workbench does not present it as actively working

### Requirement: Composer toolbar composition

The composer toolbar SHALL place the focused session's read-only agent identity (with the lock affordance) at the bottom-left, and the model chip and submit control at the bottom-right. The composer SHALL NOT render a settings entry — the app shell owns settings — SHALL NOT render any creation control, and SHALL NOT render a permission-mode control: creation-time mode choice lives in the creation dialog, and mid-session mode switching stays in the session header (webui「会话 mode 在 dashboard 可见可切」).

#### Scenario: toolbar places identity left, actions right

- **WHEN** a session is focused and the composer renders
- **THEN** the locked agent identity appears at the bottom-left and the model chip and submit control appear at the bottom-right

#### Scenario: no settings entry in the composer

- **WHEN** the operator inspects the composer toolbar
- **THEN** no settings control is rendered; settings remain reachable from the app shell's sidebar entry

#### Scenario: no mode control in the composer

- **WHEN** a session is focused and the operator inspects the composer toolbar
- **THEN** no permission-mode control is rendered — mode is visible and switchable in the session header, and the creation dialog owns the creation-time choice

### Requirement: Submit control reflects submission and turn state

The composer's submit control SHALL reflect the state of the input and the focused session's turn:

- empty input and no in-flight work: the control SHALL render disabled;
- non-empty input and no in-flight work: the control SHALL render as the enabled send affordance;
- a submission POST in flight: the control SHALL render an in-progress indication instead of the send affordance;
- the focused session has a turn in flight and the input is empty: the control SHALL render as a stop affordance, and activating it SHALL request cancellation of the session's in-flight turn;
- the focused session has a turn in flight and the input is non-empty: the control SHALL render a queued affordance, and submitting SHALL enqueue the submission via the existing turn queue without dropping or replacing pending entries;
- when the turn ends (or the cancel completes), the control SHALL return to the send affordance.

#### Scenario: empty input is disabled

- **WHEN** the input is empty and the focused session has no turn in flight
- **THEN** the submit control is disabled

#### Scenario: typed input enables send

- **WHEN** the operator types into the composer and the focused session has no turn in flight
- **THEN** the submit control becomes the enabled send affordance

#### Scenario: submission in flight shows progress

- **WHEN** a submission POST is in flight
- **THEN** the control shows an in-progress indication and returns to the send affordance when the request settles

#### Scenario: streaming with empty input offers stop

- **WHEN** the focused session is streaming a turn and the input is empty
- **THEN** the control renders as a stop affordance; activating it sends the cancel request and the control returns to the send affordance once the turn is no longer in flight

#### Scenario: streaming with text offers queueing

- **WHEN** the focused session is streaming a turn and the input is non-empty
- **THEN** the control renders a queued affordance and submitting appends the text to the pending stack

#### Scenario: cancel does not drop pending submissions

- **WHEN** the operator cancels the in-flight turn while pending submissions are queued
- **THEN** the in-flight turn is interrupted and the pending submissions remain queued

### Requirement: Workbench layout is resizable

The workbench SHALL expose two draggable boundaries: between the project rail and the main area (rail width adjustable within a bounded range), and between the conversation stage and the composer (composer height adjustable within a bounded range, the stage taking the remainder). Both dimensions SHALL persist locally in the browser and SHALL be restored on the next load without server round-trips. On narrow screens the workbench SHALL degrade to the fixed layout without draggable boundaries.

#### Scenario: dragging changes the rail width

- **WHEN** the operator drags the rail/main boundary
- **THEN** the rail width follows the pointer within the allowed range and the main area takes the remainder

#### Scenario: dragging changes the composer height

- **WHEN** the operator drags the stage/composer boundary
- **THEN** the composer height follows the pointer within the allowed range and the conversation stage keeps the remainder scrollable

#### Scenario: dimensions survive a reload

- **WHEN** the operator reloads the page after resizing
- **THEN** the rail width and composer height are restored to the persisted values

#### Scenario: narrow screens degrade

- **WHEN** the viewport is narrower than the breakpoint
- **THEN** the boundaries are not draggable and the layout falls back to the fixed arrangement

### Requirement: Workbench regions read as floating islands

The workbench SHALL visually separate its regions by layering, not by hard rules: the app background SHALL use a distinct canvas tone, and the rail, conversation stage, and composer SHALL render as rounded surfaces set off from the canvas by spacing. Draggable boundaries SHALL present no affordance at rest and SHALL reveal a drag affordance on hover. The treatment SHALL respect the design tokens and the reduced-motion preference.

#### Scenario: regions are separated by spacing and tone

- **WHEN** the workbench renders
- **THEN** the rail, conversation stage, and composer read as distinct rounded surfaces against the canvas tone rather than areas divided by hard full-height rules

#### Scenario: drag affordance appears on hover

- **WHEN** the operator hovers a draggable boundary
- **THEN** a drag affordance appears; at rest the boundary shows no persistent affordance
