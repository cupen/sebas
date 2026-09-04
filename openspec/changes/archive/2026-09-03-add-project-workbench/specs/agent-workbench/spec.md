## Purpose
Defines the project-centric agent workbench: how a project is registered and
persisted, how sessions are attributed to projects, how the turn stream marks
what arrived while the operator was away, what the composer may promise, and how
several projects' agents are driven concurrently.

## ADDED Requirements

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
