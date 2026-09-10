## ADDED Requirements

### Requirement: Rail deletion entries

Each project row in the workbench rail SHALL expose a remove action; confirming it SHALL call `POST /api/projects/{project_id}/remove`, remove the row without a reload, and migrate any live sessions under it to the Inbox group. Each session row SHALL expose a close action with `POST /api/sessions/{key}/close` semantics: inactive sessions close immediately, active sessions require an inline confirmation, and closing the focused session returns the workbench to the no-focus empty state.

#### Scenario: remove a project from the rail

- **WHEN** the operator clicks remove on a project row and confirms
- **THEN** the project is removed from the rail, and live sessions under it appear in the Inbox group without being killed

#### Scenario: close a dormant session from the rail

- **WHEN** the operator clicks close on a dormant session row
- **THEN** the session is closed immediately and the row disappears without a confirmation dialog

#### Scenario: closing a working session asks first

- **WHEN** the operator clicks close on a session whose child is running
- **THEN** an inline confirmation is shown; only on confirm is the close sent

#### Scenario: closing the focused session clears the stage

- **WHEN** the operator closes the currently focused session
- **THEN** the workbench returns to the no-focus empty state and the composer re-enters creation mode

### Requirement: Placeholder session is immediately writable

After the operator creates a 0-turn placeholder session (rail `+` button), the workbench composer SHALL enter follow-up mode for that session: the first composer submission SHALL be delivered to the placeholder via `POST /api/sessions/{key}/message` and SHALL NOT spawn another session. A placeholder session SHALL be identifiable by an explicit awaiting-first-prompt marker so the first message reliably triggers spawn regardless of which agent id was chosen.

#### Scenario: first message reaches the placeholder session

- **WHEN** the operator creates a placeholder session from the rail and submits a composer message
- **THEN** the message is posted to the placeholder's message endpoint, the session spawns its child, and no second session is created

#### Scenario: composer shows follow-up mode after placeholder creation

- **WHEN** a placeholder session has just been created and focused
- **THEN** the composer renders follow-up mode (read-only agent label, no agent selector) for that session

#### Scenario: placeholder spawn works for any valid agent

- **WHEN** a placeholder was created with `agent = "claudecode"`, `"codex"`, or `"native"` and receives its first message
- **THEN** the child spawns under that agent; the spawn does not depend on optional fields being present on the mapping

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
