## Purpose

Defines the directory-picker project registration, zero-prompt session creation, session archiving with retention expiry, and the History/Inbox split in the workbench sidebar — completing the project and session action surface that the initial workbench implementation left as gaps.

## Requirements


### Requirement: Add project via directory picker

The workbench SHALL provide a modal dialog with a server-side directory tree browser and a manual path input, either of which SHALL register a project directory. The tree browser SHALL lazy-load subdirectory listings on expand from `GET /api/fs/browse-dirs?path=...&root=...`, scoped to the server-side work root. The tree SHALL open at the server's default work directory (the listing rooted there, its canonical path displayed so the operator can see the starting scope), and expanding any node SHALL list its subdirectories without error — including on Windows, where previously the path round-trip failed with a 400. The registered project name SHALL be the directory's basename. The system SHALL probe the directory for a git branch after registration to determine directory accessibility; the branch name SHALL NOT be displayed in the project row. The manual path input SHALL NOT be bounded by the tree's root scope.

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
- **THEN** the project row does not show a branch name, while the probe result still drives the directory-accessibility marking

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

Each project row SHALL expose its remove action inside the row's overflow (`...`) menu. When the project has at least one non-archived session, the removal SHALL be rejected — the backend SHALL refuse the remove request and the dialog SHALL state that sessions must be archived or closed first, naming the session count. When no non-archived sessions remain, confirming the dialog SHALL call `POST /api/projects/{id}/remove` and the project row SHALL disappear from the rail without a page reload. A backend rejection SHALL be presented inline in the dialog, with the row retained.

#### Scenario: remove a project from the rail

- **WHEN** the operator removes a project that has no non-archived sessions and confirms the dialog
- **THEN** the remove endpoint is called and the project disappears from the rail

#### Scenario: removal is blocked while sessions exist

- **WHEN** the operator attempts to remove a project that still has non-archived sessions
- **THEN** the removal is refused with a message naming the session count and directing the operator to archive or close them first

#### Scenario: removal rejection surfaces inline

- **WHEN** the backend rejects the removal
- **THEN** the dialog presents the typed error inline and the project row stays in the rail

#### Scenario: cancel leaves the registry untouched

- **WHEN** the operator opens the remove confirmation and cancels
- **THEN** no request is sent and the project stays registered

### Requirement: Session close from the rail

Each session row in the workbench rail SHALL expose a close action inside the row's overflow (`...`) menu, with `POST /api/sessions/{key}/close` semantics (kill the child when active, drop the mapping). Closing an inactive session (dormant/done/failed) SHALL act immediately; closing an active session (starting/queued/working) SHALL require an inline confirmation. Closing the focused session SHALL return the workbench to the no-focus empty state.

#### Scenario: close a dormant session from the rail

- **WHEN** the operator picks Close from a dormant session row's overflow menu
- **THEN** the session is closed immediately, the row disappears, and no confirmation is required

#### Scenario: closing a working session asks first

- **WHEN** the operator picks Close on a session whose child is running
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

### Requirement: Row action consolidation

A project row SHALL expose exactly two hover-revealed affordances, in order: an overflow (`...`) menu followed by the "New session" (`+`) button. A session row SHALL expose a single hover-revealed overflow (`...`) menu and no add affordance. All row affordances SHALL be hidden until the row is hovered or keyboard-focused. The overflow menus SHALL be implemented as dropdowns so future actions can be added without new row buttons; the project menu currently contains only Remove.

#### Scenario: project row shows menu then plus on hover

- **WHEN** the operator hovers or keyboard-focuses a project row
- **THEN** the `...` menu and the `+` button appear in that order, and are hidden again when the row is no longer hovered or focused

#### Scenario: session row has only the overflow menu

- **WHEN** the operator hovers a session row
- **THEN** only a `...` menu appears; the row provides no add button, and the menu contains Archive and Close

#### Scenario: project overflow menu opens the remove dialog

- **WHEN** the operator opens a project row's `...` menu and picks Remove
- **THEN** the remove confirmation dialog opens for that project

### Requirement: Session rows are named by the first prompt

A rail session row SHALL display the preview of the session's first user message as its name. A session that has not received any user message yet (a zero-turn placeholder) SHALL fall back to its short session identifier. Names longer than the implementation-defined display cap SHALL be truncated with an ellipsis, while the row's hover title SHALL carry the full first message. When the first message is sent to a placeholder session, its name SHALL update from the identifier to the message preview without a page reload. The rail's session dialogs SHALL name the session by the same label as its row.

#### Scenario: named by the first message

- **WHEN** a session has received a first user message
- **THEN** its rail row shows a preview of that message instead of the session identifier

#### Scenario: placeholder falls back to the identifier

- **WHEN** a zero-turn placeholder session has no user message yet
- **THEN** its rail row shows the short session identifier as the name

#### Scenario: long first message is truncated

- **WHEN** a session's first user message exceeds the display cap
- **THEN** the row shows a truncated name ending with an ellipsis, and hovering the row reveals the full message via its title

#### Scenario: placeholder name updates after the first message

- **WHEN** the operator sends the first message into a placeholder session
- **THEN** the row's name becomes the message preview without a page reload
