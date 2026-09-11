## MODIFIED Requirements

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

## ADDED Requirements

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
