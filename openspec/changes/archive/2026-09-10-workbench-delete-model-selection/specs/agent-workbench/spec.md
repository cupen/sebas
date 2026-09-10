## ADDED Requirements

### Requirement: Rail project removal entry

Each project row in the workbench rail SHALL expose a remove action (hover-revealed, consistent with the existing row-action affordance). Triggering it SHALL open a confirmation dialog that names the project and states that live sessions under it keep running and migrate to the Inbox. Confirming SHALL call `POST /api/projects/{path}/remove` and remove the row from the rail without a page reload; the dialog SHALL present the typed error inline when the backend rejects the removal. Cancelling SHALL leave the registry untouched.

#### Scenario: remove a project from the rail

- **WHEN** the operator clicks the remove button on a project row and confirms the dialog
- **THEN** `POST /api/projects/{path}/remove` is called, the project disappears from the rail, and no page reload is required

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

Each session row in the workbench rail (project groups and Inbox) SHALL expose a close action alongside the existing archive button, with the same semantics as `POST /api/sessions/{key}/close` (kill the child when active, drop the mapping). Closing SHALL require no confirmation for inactive (dormant/done/failed) sessions and SHALL require an inline confirmation for active (starting/queued/working) sessions. When the closed session was the focused one, the workbench SHALL return to the no-focus empty state.

#### Scenario: close a dormant session from the rail

- **WHEN** the operator clicks the close button on a dormant session row
- **THEN** the session is closed immediately, the row disappears, and no confirmation dialog is shown

#### Scenario: closing a working session asks first

- **WHEN** the operator clicks the close button on a session whose child is still running
- **THEN** an inline confirmation is shown first, and only on confirm is the close request sent

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

## MODIFIED Requirements

### Requirement: Composer promises only what the process can do

The composer SHALL deliver every accepted submission to the core over the session channel, in every process configuration, and SHALL NOT accept a message it cannot deliver. When the channel reports the core unreachable, the composer SHALL render disabled with that cause stated, and SHALL become enabled again on reconnection without a manual reload.

The composer's mode (follow-up vs creation) SHALL be derived from the webui-side focused-session pointer: a focused session puts the composer in follow-up mode targeting that session; no focused session (or an explicit "new session" request) puts it in creation mode. Focusing a session — via the `/api/sessions/{key}/switch` endpoint or by visiting the session's deep-link page — SHALL update the pointer so that a subsequent composer submission follows the focused session rather than spawning a new one.

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
