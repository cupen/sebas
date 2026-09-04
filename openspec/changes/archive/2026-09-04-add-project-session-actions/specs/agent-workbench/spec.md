## ADDED Requirements

### Requirement: Add project via directory browser

The workbench SHALL provide a modal dialog with a server-side directory browser and a manual path input, either of which SHALL register a project directory. The registered project name SHALL be the directory's basename. The system SHALL probe the directory for a git branch after registration.

#### Scenario: add project via directory browser

- **WHEN** the operator clicks the "Browse Directories…" button in the Add Project dialog
- **THEN** the system fetches a directory listing from `GET /api/fs/browse?path=…`, presents it in a navigable tree, and the operator selects a directory, which registers it as a project

#### Scenario: add project via manual path

- **WHEN** the operator types a path into the manual input field and clicks "Add project"
- **THEN** the path is validated and registered, with the same behaviour as the browser path

#### Scenario: project name from directory name

- **WHEN** a project is registered at `/home/user/work/my-repo`
- **THEN** the project name is `my-repo`

### Requirement: New session without prompt

The workbench SHALL support creating a 0-turn placeholder session without requiring a prompt. The placeholder SHALL appear in the session list immediately and SHALL be activated. An ACP child SHALL NOT be spawned until the first message is sent. Each project row SHALL have a dedicated "New session" button.

#### Scenario: create empty session from project

- **WHEN** the operator clicks the `+` button on a project row
- **THEN** a new session with zero turns is created, the project is selected, the session is activated, and the composer is ready for the first message

#### Scenario: first message spawns the child

- **WHEN** the operator sends a message into a zero-turn placeholder session
- **THEN** the system spawns the ACP child and the session transitions to working

### Requirement: Session archive

Each session row SHALL have an archive button that moves the session to the History group. An archived session SHALL be read-only — the operator cannot send messages into it, cannot close it, and cannot switch to it as the active session. An archived session SHALL be restorable to its original project by clicking it in the History group.

#### Scenario: archive a session

- **WHEN** the operator clicks the archive button on a session row
- **THEN** the session is moved to the History group, marked as read-only, and the operator cannot interact with it

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

The system SHALL permanently delete archived sessions whose `archived_at` timestamp is older than the configured retention period. The default retention SHALL be 30 days, configurable via `[webui] archive_retention_days` in the config file. Expired sessions SHALL be removed on WebUI startup and on every session list request.

#### Scenario: expired session cleaned up

- **WHEN** the WebUI starts or the session list is requested and an archived session exceeds the retention period
- **THEN** that session is permanently removed from the archive list

#### Scenario: retention configured

- **WHEN** `[webui] archive_retention_days = 7` is set in the config
- **THEN** archived sessions older than 7 days are removed at startup and on list requests