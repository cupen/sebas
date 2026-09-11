## MODIFIED Requirements

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

### Requirement: Composer promises only what the process can do

The composer SHALL deliver every accepted submission to the core over the session channel, in every process configuration, and SHALL NOT accept a message it cannot deliver. When the channel reports the core unreachable, the composer SHALL render disabled with that cause stated, and SHALL become enabled again on reconnection without a manual reload.

The composer's mode (follow-up vs creation) SHALL be derived from the webui-side focused-session pointer: a focused session puts the composer in follow-up mode targeting that session; no focused session (or an explicit "new session" request) puts it in creation mode. Focusing a session — via the `/api/sessions/{key}/switch` endpoint or by visiting the session's deep-link page — SHALL update the pointer so that a subsequent composer submission follows the focused session rather than spawning a new one.

In creation mode the composer SHALL require an explicit agent choice drawn from `/api/agents`; there is no implicit or "null" agent. The selector SHALL preselect the current project's remembered default agent when one exists. A creation-mode submission SHALL be bound to an explicitly selected project; the composer SHALL NOT offer project-less (inbox) binding.

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
- **THEN** the composer renders creation mode, and a submission spawns a new session bound to the selected project

#### Scenario: creation mode requires an explicit agent

- **WHEN** the composer is in creation mode and the operator has not chosen an agent
- **THEN** the submit control is disabled until an agent is chosen from the `/api/agents` list

#### Scenario: creation mode requires an explicit project

- **WHEN** the composer is in creation mode and no project is selected
- **THEN** the submit control is disabled until the operator selects a project; no inbox binding is offered

### Requirement: Session origin is visible

Each session SHALL show whether it originated in Feishu or in the workbench, so
the operator can tell which surface a conversation started on. **Deferred**：
origin 标签尚未在任何视图渲染（会话行携带 `channel` 字段，UI 未消费）——当前
仅能经会话键形状间接推断；落一个 origin 徽标属小改动。

#### Scenario: origin shown per session

- **WHEN** a session is displayed
- **THEN** its origin is stated as Feishu or workbench（deferred：当前渲染缺位，
  见上）
