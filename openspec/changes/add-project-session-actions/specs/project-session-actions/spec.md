## Purpose

Defines the directory-picker project registration, zero-prompt session creation, session archiving with retention expiry, and the History/Inbox split in the workbench sidebar — completing the project and session action surface that the initial workbench implementation left as gaps.

## ADDED Requirements

### Requirement: Add project via directory picker

The workbench SHALL provide a modal dialog with a server-side directory browser (`GET /api/fs/browse`) and a manual path input, either of which SHALL register a project directory. The registered project name SHALL be the directory's basename. The system SHALL probe the directory for a git branch after registration and display it in the project row.

#### Scenario: add project via directory browser

- **WHEN** the operator clicks the "Browse Directories…" button in the Add Project dialog
- **THEN** the system fetches a directory listing from `GET /api/fs/browse?path=…`, presents it in a navigable tree, and the operator selects a directory, which registers it as a project

#### Scenario: add project via manual path

- **WHEN** the operator types a path into the manual input field and clicks "Add project"
- **THEN** the path is validated and registered, with the same behaviour as the browser path

#### Scenario: project name from directory name

- **WHEN** a project is registered at `/home/user/work/my-repo`
- **THEN** the project name is `my-repo`

#### Scenario: git branch shown after registration

- **WHEN** a project is registered and the directory is a git repository
- **THEN** the project row shows the current branch name

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