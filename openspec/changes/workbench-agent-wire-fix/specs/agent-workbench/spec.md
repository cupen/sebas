## ADDED Requirements

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

### Requirement: Rail deletion entries

Each project row in the workbench rail SHALL expose a remove action; confirming it SHALL call the project-remove endpoint, remove the row without a reload, and migrate any live sessions to the Inbox group. Each session row SHALL expose a close action with `POST /api/sessions/{key}/close` semantics: inactive sessions close immediately, active sessions require an inline confirmation, and closing the focused session returns the workbench to the no-focus empty state.

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

## MODIFIED Requirements

### Requirement: Composer promises only what the process can do

The composer SHALL deliver every accepted submission to the core over the session channel, in every process configuration, and SHALL NOT accept a message it cannot deliver. When the channel reports the core unreachable, the composer SHALL render disabled with that cause stated, and SHALL become enabled again on reconnection without a manual reload.

The composer's mode (follow-up vs creation) SHALL be derived from the webui-side focused-session pointer: a focused session puts the composer in follow-up mode targeting that session; no focused session (or an explicit "new session" request) puts it in creation mode. Focusing a session — via the switch endpoint or by visiting the session's deep-link page — SHALL update the pointer so that a subsequent composer submission follows the focused session rather than spawning a new one.

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
