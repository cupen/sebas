# agent-workbench delta — agent-task-center

## ADDED Requirements

### Requirement: Stage area gains a "Tasks" tab alongside "Conversation"

The workbench stage SHALL offer two view tabs when a project is selected: `Conversation` (the existing session-focused view) and `Tasks` (the new task board). The rail SHALL be unchanged: it continues to list projects and their sessions; tasks SHALL NOT appear in the rail. Switching between `Conversation` and `Tasks` SHALL NOT close, interrupt, or re-route any session, and SHALL NOT change the focused-session pointer. The tab state SHALL be per-project: switching projects preserves each project's last-active tab.

#### Scenario: Tabs are visible when a project is selected

- **WHEN** an operator selects a project in the rail
- **THEN** the stage header shows `Conversation` and `Tasks` tabs, with the project's last-active tab selected

#### Scenario: Switching tabs does not affect sessions

- **WHEN** an operator switches from `Conversation` to `Tasks` while a session is actively streaming
- **THEN** the session continues uninterrupted, no focus change occurs, and switching back to `Conversation` shows the session exactly as it was

#### Scenario: Tab state is per-project

- **WHEN** project A is on the `Tasks` tab and project B is on `Conversation`, and the operator switches between them
- **THEN** each project renders its own last-active tab

### Requirement: Task detail offers "Start a session for this task"

The task detail page SHALL expose a button that opens the existing session-creation dialog (agent selector, model selector, permission-mode selector), retitled to "Advance task bd-XX: <task title>". The dialog's behavior SHALL be unchanged except that confirming SHALL call `POST /api/sessions` with the additional `task_id` field. On success, the stage SHALL switch to the `Conversation` tab focused on the newly created session, and the new session row SHALL appear in the rail under the same project.

#### Scenario: Dialog opens with the task context

- **WHEN** an operator clicks "Start a session for this task" on task bd-42
- **THEN** the existing creation dialog opens with title "Advance task bd-42: <task title>" and the agent/model/mode selectors behave exactly as in the rail creation flow

#### Scenario: Confirming creates a linked session

- **WHEN** the operator confirms the dialog
- **THEN** the create-session request carries `task_id = "bd-42"`, the new session is created and focused, and the stage switches to `Conversation`

#### Scenario: Cancel creates nothing

- **WHEN** the operator cancels the dialog
- **THEN** no session is created, the task detail page remains visible, and no focus change occurs

### Requirement: Session rows show the linked task as a back-link

When a session has a non-empty `task_id`, the session detail view SHALL render a back-link "Task bd-XX" near the session header. Clicking the link SHALL switch the stage to the `Tasks` tab and open that task's detail page. The rail session row itself SHALL NOT display the task link (rail density is preserved).

#### Scenario: Back-link appears on linked sessions

- **WHEN** an operator opens a session whose `task_id` is set
- **THEN** the session header area shows a "Task bd-XX" link

#### Scenario: Clicking the back-link opens the task

- **WHEN** an operator clicks the "Task bd-XX" link
- **THEN** the stage switches to the `Tasks` tab and renders task bd-XX's detail page

#### Scenario: Unlinked sessions have no back-link

- **WHEN** an operator opens a session with no `task_id`
- **THEN** no "Task …" link is rendered in the session header
