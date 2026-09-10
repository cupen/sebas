## ADDED Requirements

### Requirement: Project removal from the rail

Each project row in the workbench rail SHALL expose a remove action (hover-revealed). Triggering it SHALL open a confirmation dialog that names the project and states that live sessions under it keep running and migrate to the Inbox. Confirming SHALL call `POST /api/projects/{path}/remove`; the project row SHALL disappear from the rail without a page reload. A backend rejection SHALL be presented inline in the dialog, with the row retained.

#### Scenario: remove a project from the rail

- **WHEN** the operator clicks the remove button on a project row and confirms the dialog
- **THEN** the remove endpoint is called, the project disappears from the rail, and any live sessions under it appear in the Inbox group

#### Scenario: removal rejection surfaces inline

- **WHEN** the backend rejects the removal
- **THEN** the dialog presents the typed error inline and the project row stays in the rail

#### Scenario: cancel leaves the registry untouched

- **WHEN** the operator opens the remove confirmation and cancels
- **THEN** no request is sent and the project stays registered

### Requirement: Session close from the rail

Each session row in the workbench rail (project groups and Inbox) SHALL expose a close action alongside the archive button, with `POST /api/sessions/{key}/close` semantics (kill the child when active, drop the mapping). Closing an inactive session (dormant/done/failed) SHALL act immediately; closing an active session (starting/queued/working) SHALL require an inline confirmation. Closing the focused session SHALL return the workbench to the no-focus empty state.

#### Scenario: close a dormant session from the rail

- **WHEN** the operator clicks the close button on a dormant session row
- **THEN** the session is closed immediately, the row disappears, and no confirmation is required

#### Scenario: closing a working session asks first

- **WHEN** the operator clicks the close button on a session whose child is running
- **THEN** an inline confirmation is shown first; only on confirm is the close sent

#### Scenario: closing the focused session clears the stage

- **WHEN** the operator closes the currently focused session
- **THEN** the workbench stage returns to the no-focus empty state and the composer re-enters creation mode

### Requirement: Placeholder session is immediately writable

After the operator creates a 0-turn placeholder session (rail `+` button), the workbench composer SHALL enter follow-up mode for that session: the first composer submission SHALL be delivered to the placeholder via `POST /api/sessions/{key}/message` and SHALL NOT spawn another session.

#### Scenario: first message reaches the placeholder session

- **WHEN** the operator creates a placeholder session from the rail and submits a composer message
- **THEN** the message is posted to the placeholder's message endpoint, the session spawns its child, and no second session is created

#### Scenario: composer shows follow-up mode after placeholder creation

- **WHEN** a placeholder session has just been created and focused
- **THEN** the composer renders follow-up mode (read-only agent label, no execution-backend dropdown) for that session
