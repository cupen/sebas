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
state and SHALL NOT be recorded server-side.

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

### Requirement: Composer promises only what the process can do

The composer SHALL deliver every accepted submission to the core over the session
channel, in every process configuration, and SHALL NOT accept a message it cannot
deliver. When the channel reports the core unreachable, the composer SHALL render
disabled with that cause stated, and SHALL become enabled again on reconnection
without a manual reload.

#### Scenario: composer drives in either configuration

- **WHEN** the workbench runs detached or in-process and the core is reachable
- **THEN** the composer is enabled and a sent message reaches the agent in both

#### Scenario: core unreachable disables with a cause

- **WHEN** the session channel reports the core unreachable
- **THEN** the composer is disabled and states that the core is not connected,
  rather than presenting an enabled control

#### Scenario: no silent discard

- **WHEN** the core is unreachable
- **THEN** no code path accepts a composer submission and reports success

#### Scenario: recovery needs no reload

- **WHEN** the core returns after being unreachable while the page stays open
- **THEN** the composer becomes enabled again without the operator reloading

### Requirement: Session origin is visible

Each session SHALL show whether it originated in Feishu or in the workbench, so
the operator can tell which surface a conversation started on.

#### Scenario: origin shown per session

- **WHEN** a session is displayed
- **THEN** its origin is stated as Feishu or workbench

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
