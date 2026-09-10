## Purpose

Defines the directory-picker project registration, zero-prompt session creation, session archiving with retention expiry, and the History/Inbox split in the workbench sidebar — completing the project and session action surface that the initial workbench implementation left as gaps.

## Requirements

### Requirement: Add project via directory picker

The workbench SHALL provide a modal dialog with a server-side directory tree browser and a manual path input, either of which SHALL register a project directory. The tree browser SHALL lazy-load subdirectory listings on expand from `GET /api/fs/browse-dirs?path=...&root=...`, scoped to the server-side work root. The tree SHALL open at the server's default work directory (the listing rooted there, its canonical path displayed so the operator can see the starting scope), and expanding any node SHALL list its subdirectories without error — including on Windows, where previously the path round-trip failed with a 400. The registered project name SHALL be the directory's basename. The system SHALL probe the directory for a git branch after registration and display it in the project row. The manual path input SHALL NOT be bounded by the tree's root scope.

#### Scenario: add project via directory browser

- **WHEN** the operator opens the Add Project dialog and expands a node in the directory tree
- **THEN** the system fetches subdirectory listings from `GET /api/fs/browse-dirs` on demand, presents them lazily, and the operator selects a directory, which registers it as a project

#### Scenario: tree opens at the server work directory

- **WHEN** the Add Project dialog opens
- **THEN** the directory tree's top level lists the contents of the server's default work directory and shows its canonical path, without requiring the operator to navigate down from a filesystem root

#### Scenario: expanding a node lists subdirectories without error

- **WHEN** the operator expands a directory node in the tree on any supported platform (including Windows)
- **THEN** the node's subdirectories are listed beneath it; a failed listing shows an inline error in the tree instead of a silent empty node

#### Scenario: add project via manual path

- **WHEN** the operator types a path into the manual input field and clicks "Add project"
- **THEN** the path is validated and registered, with the same behaviour as the browser path

#### Scenario: manual path outside the tree root still registers

- **WHEN** the operator types a valid directory path that lies outside the tree's starting scope and submits it
- **THEN** the directory is registered as a project

#### Scenario: project name from directory name

- **WHEN** a project is registered at `/home/user/work/my-repo`
- **THEN** the project name is `my-repo`

#### Scenario: git branch shown after registration

- **WHEN** a project is registered and the directory is a git repository
- **THEN** the project row shows the current branch name

#### Scenario: empty directory is not expandable

- **WHEN** a directory has no subdirectories
- **THEN** it has no expand chevron, stays aligned with other rows, and clicking it only selects the path without a loading state

### Requirement: New session without prompt

The workbench SHALL support creating a 0-turn placeholder session without requiring a prompt. The placeholder SHALL appear in the session list immediately and SHALL be activated. An ACP child SHALL NOT be spawned until the first message is sent. The project row SHALL have a dedicated "New session" button.

#### Scenario: create empty session from project

- **WHEN** the operator clicks the `+` button on a project row
- **THEN** a new session with zero turns is created, the project is selected, the session is activated, and the composer is ready for the first message

#### Scenario: first message spawns the child

- **WHEN** the operator sends a message into a zero-turn placeholder session
- **THEN** the system spawns the ACP child and the session transitions to working

#### Scenario: empty session created via API

- **WHEN** `POST /api/sessions` is called without a `prompt` field
- **THEN** a placeholder session is created and the response includes its key, with status `spawning` and no turn entries

### Requirement: Session archive

Each session row SHALL have an archive button that moves the session to the History group. An archived session SHALL be read-only — the operator cannot send messages into it, cannot close it, and cannot switch to it as the active session. An archived session SHALL be restorable to its original project.

#### Scenario: archive a session

- **WHEN** the operator clicks the archive button on a session row
- **THEN** the session is moved to the History group, marked as read-only, and the operator cannot interact with it

#### Scenario: archived session is read-only

- **WHEN** the operator attempts to send a message to an archived session
- **THEN** the system rejects the message with a 400 response stating the session is archived

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

The system SHALL permanently delete archived sessions whose `archived_at` timestamp is older than the configured retention period. The default retention SHALL be 30 days, configurable via `[webui] archive_retention_days` in the config file. Expired sessions SHALL be removed on WebUI startup and on every session list request, with no operator-facing notification.

#### Scenario: expired session cleaned up

- **WHEN** the WebUI starts or the session list is requested and an archived session exceeds the retention period
- **THEN** that session is permanently removed from the archive list and is no longer shown

#### Scenario: retention configured

- **WHEN** `[webui] archive_retention_days = 7` is set in the config
- **THEN** archived sessions older than 7 days are removed at startup and on list requests

#### Scenario: within retention

- **WHEN** an archived session is within the retention period
- **THEN** it remains in the History group and is listed

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
