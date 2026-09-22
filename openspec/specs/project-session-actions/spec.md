## Purpose

Defines the directory-picker project registration, zero-prompt session creation, session archiving with retention expiry, and the History/Inbox split in the workbench sidebar — completing the project and session action surface that the initial workbench implementation left as gaps.

## Requirements

### Requirement: Add project via directory picker

The workbench SHALL provide a modal dialog with a server-side directory tree browser and a manual path input, either of which SHALL register a project directory. The tree browser SHALL lazy-load subdirectory listings on expand from `GET /api/fs/browse-dirs?path=...&root=...`, scoped to the workspace root. The tree SHALL open at the workspace root (the listing rooted there, its canonical path displayed so the operator can see the starting scope), and expanding any node SHALL list its subdirectories without error — including on Windows, where previously the path round-trip failed with a 400. The tree SHALL NOT offer a directory that resolves onto the built-in system-directory denylist. The registered project name SHALL be the directory's basename. The system SHALL probe the directory for a git branch after registration to determine directory accessibility; the branch name SHALL NOT be displayed in the project row. The manual path input SHALL be bounded by the workspace root: a path that resolves outside it SHALL be rejected with an out-of-scope error, identically to the browser path. A submitted path that resolves onto a built-in system directory SHALL likewise be refused — through either entry — with an error naming the submitted path shown inline in the dialog.

#### Scenario: add project via directory browser

- **WHEN** the operator opens the Add Project dialog and expands a node in the directory tree
- **THEN** the system fetches subdirectory listings from `GET /api/fs/browse-dirs` on demand, presents them lazily, and the operator selects a directory, which registers it as a project

#### Scenario: tree opens at the server work directory

- **WHEN** the Add Project dialog opens
- **THEN** the directory tree's top level lists the contents of the workspace root (replacing the former server work directory start) and shows its canonical path, without requiring the operator to navigate down from a filesystem root

#### Scenario: expanding a node lists subdirectories without error

- **WHEN** the operator expands a directory node in the tree on any supported platform (including Windows)
- **THEN** the node's subdirectories are listed beneath it; a failed listing shows an inline error in the tree instead of a silent empty node

#### Scenario: add project via manual path

- **WHEN** the operator types a path into the manual input field and clicks "Add project"
- **THEN** the path is validated and registered, with the same behaviour as the browser path

#### Scenario: manual path outside the tree root still registers

- **WHEN** the operator types a valid directory path that resolves outside the workspace root and submits it
- **THEN** the registration is rejected with an out-of-scope error and no project is created — the former out-of-scope tolerance is revoked by this change

#### Scenario: system directory via manual input is rejected

- **WHEN** the operator types a path that resolves onto a built-in system directory (for example `/usr`, or `C:\Windows`) and submits it
- **THEN** the registration is rejected, the dialog shows the typed error naming the submitted path, and no project is created

#### Scenario: the tree does not offer denylisted directories

- **WHEN** a browsed listing would contain a directory that resolves onto the built-in denylist
- **THEN** that directory does not appear as a selectable tree node, so it cannot be picked

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

The workbench SHALL support creating a 0-turn placeholder session without requiring a prompt. The placeholder SHALL appear in the session list immediately and SHALL be activated. An ACP child SHALL NOT block creation: focusing a placeholder session that has no live child SHALL start the child in the background — resuming the recorded conversation when the session mapping allows it — and the first message SHALL also start the child if focus never did. The project row SHALL have a dedicated "New session" button. A failed background start SHALL NOT remove the placeholder.

#### Scenario: create empty session from project

- **WHEN** the operator clicks the `+` button on a project row and confirms the creation dialog (agent chosen, no prompt entered)
- **THEN** a new session with zero turns is created, the project is selected, the session is activated, and the composer is ready for the first message

#### Scenario: first message spawns the child

- **WHEN** the operator sends a message into a zero-turn placeholder session whose child was never started (or has not finished starting)
- **THEN** the system spawns the ACP child and the session transitions to working

#### Scenario: empty session created via API

- **WHEN** `POST /api/sessions` is called without a `prompt` field
- **THEN** a placeholder session is created and the response includes its key, with status `spawning` and no turn entries

#### Scenario: focusing the placeholder starts the child with resume

- **WHEN** the operator focuses a placeholder session that has no live child and the session carries a recorded conversation mapping
- **THEN** the child starts in the background and resumes that conversation without the operator sending a prompt first

#### Scenario: failed background start keeps the placeholder

- **WHEN** the background start of a placeholder's child fails
- **THEN** the placeholder stays in the session list and the failure is stated, not silently swallowed

### Requirement: Session archive

The rail session row's overflow menu SHALL be the operator-facing archive entry: archiving moves the session to the History group and carries the close semantics (the child is killed if active; the confirm dialog warns about pending submissions that will be discarded). The focused session's header SHALL render no action buttons and no navigation link. An archived session SHALL be read-only — the operator cannot send messages into it, cannot close it, and cannot switch to it as the active session. An archived session SHALL be restorable to its original project. The archive entry SHALL carry the session's identity — agent binding (`agent_kind`), desired permission mode, and the current/available model catalog — and restoring SHALL rebuild the session with that identity intact, so the restored session continues with the same agent and model surface it had before archiving. Legacy archive entries without identity fields SHALL restore with the existing fallbacks and the UI SHALL present that fallback honestly. Restoring SHALL remove the History entry and rebuild the session row **as one atomic outcome**: after a successful restore the session MUST be present in the session list under its original project with its full transcript, and the archive MUST no longer hold the entry. An implementation that consumes the archive entry without rebuilding the session — leaving the data reachable nowhere — SHALL be treated as data loss and is non-conformant.

Clicking an archived session in the History group SHALL open a read-only archived view of that session in the main area and SHALL NOT restore it. The archived view SHALL present an explicit restore action — separate from the click target — which, after the operator confirms a restore dialog, restores the session to its original project, makes it writable, and activates it. Every restore attempt SHALL surface an outcome notification (success or failure). A restored session whose original project is not currently registered SHALL still surface its whereabouts: the outcome notification SHALL state which project path it was restored to, and the session SHALL become reachable through that project once registered.

#### Scenario: archive a session

- **WHEN** the operator picks archive in a rail session row's overflow menu and confirms
- **THEN** the session is moved to the History group, marked as read-only, and cannot be interacted with

#### Scenario: archived session is read-only

- **WHEN** the operator attempts to send a message to an archived session
- **THEN** the system rejects the message with a 400 response stating the session is archived

#### Scenario: clicking an archived session views it read-only

- **WHEN** the operator clicks an archived session in the History group
- **THEN** the main area opens a read-only view of that session's conversation, the History group is unchanged, and no restore happens

#### Scenario: restore is an explicit confirmed action

- **WHEN** the operator activates the restore action in the archived view and confirms the restore dialog
- **THEN** the session is restored to its original project, becomes writable, is activated, and a success notification states where it was restored to

#### Scenario: restore failure is surfaced

- **WHEN** a restore attempt fails
- **THEN** a failure notification names the session and the cause, and the session remains archived

#### Scenario: restore archived session

- **WHEN** the operator activates the explicit restore action in the archived view and confirms
- **THEN** the session is restored to its original project, becomes writable, and is activated — clicking the History row alone never restores

#### Scenario: restore preserves the transcript

- **WHEN** an archived session holding N transcript entries is restored
- **THEN** the rebuilt session exposes the same N entries via the session detail API, the session is listed under its original project, and the History group no longer lists it
- **AND** no state exists in which the archive entry is consumed while the session is absent from the session list

#### Scenario: restore into an unregistered project is not silent

- **WHEN** the operator restores a session whose original project path is not a registered project
- **THEN** the outcome notification states the project path it was restored to, and no archived entry disappears without a stated outcome

#### Scenario: focused session header offers no actions

- **WHEN** a session is focused and the operator looks at the session header
- **THEN** the header shows display information only — no Archive button, no Close button, and no "All sessions" link

#### Scenario: 恢复保留 agent 身份与模型面

- **WHEN** a session bound to a non-default agent with a selected model is archived and then restored
- **THEN** the restored session reports the same `agent_kind`, desired mode, and current/available models, and the next turn runs on that agent

#### Scenario: 旧归档条目如实回退

- **WHEN** a legacy archive entry without the identity fields is restored
- **THEN** the session comes back on the default agent with honest fallback display instead of a fabricated identity

### Requirement: History group is the archive

The History group SHALL contain only archived sessions, listed newest-first by archive time. The rail SHALL NOT render an Inbox group: sessions with no project directory — whose only source is the Feishu channel, since webui creation paths always bind a project — SHALL NOT be listed in the rail; they remain accessible through their originating surface (Feishu) and the sessions API. The History group SHALL show the total count of archived sessions and be collapsible.

#### Scenario: History holds only archived sessions

- **WHEN** the History group is expanded
- **THEN** every session listed has been archived

#### Scenario: History is sorted newest-first

- **WHEN** sessions are archived at different times
- **THEN** the History group lists them in descending order of archive time

#### Scenario: unbound sessions stay out of the rail

- **WHEN** a session has no project directory (a Feishu-originated session)
- **THEN** it appears in no rail group — the Inbox group no longer exists — and History does not list it either; its conversation continues on its originating surface

### Requirement: Archive expiry

The system SHALL permanently delete archived sessions whose `archived_at` timestamp is older than the configured retention period. The default retention SHALL be 30 days, configurable via `[service.webui] archive_retention_days` in the config file. Expired sessions SHALL be removed on WebUI startup and on every session list request, with no operator-facing notification.

#### Scenario: expired session cleaned up

- **WHEN** the WebUI starts or the session list is requested and an archived session exceeds the retention period
- **THEN** that session is permanently removed from the archive list and is no longer shown

#### Scenario: retention configured

- **WHEN** `[service.webui] archive_retention_days = 7` is set in the config
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

A rail session row SHALL display a name for the session: if the operator has set a label for the session, the label SHALL be shown; otherwise the preview of the session's first user message SHALL be used. A session that has neither a label nor any user message (a zero-turn placeholder) SHALL fall back to its short session identifier. Names longer than the implementation-defined display cap SHALL be truncated with an ellipsis, while the row's hover title SHALL carry the full text. When the first message is sent to a placeholder session, its name SHALL update from the identifier to the message preview without a page reload — unless an operator label is set, which SHALL stay stable. The operator SHALL be able to set and change the label from the rail (row actions), and the rail's session dialogs SHALL name the session by the same label as its row.

Restoring an archived session SHALL preserve the naming inputs captured at archive time: the first-prompt preview and the operator label (if any) SHALL survive the restore, so the restored row is named exactly as before archival and never regresses to the raw session identifier while a naming source exists.

A label write that the server accepts SHALL reach connected clients as a session update event, and every connected client's rail SHALL re-render that row's name from the event without a page reload — the same liveness the first-message preview already enjoys, regardless of whether the write came from the rail dialog or the API. The rename dialog SHALL persist exactly the value it displays: submitting the dialog SHALL deliver the shown text to the label write path, and a confirmed save SHALL never leave the stored label unchanged while the dialog rendered a non-empty value.

#### Scenario: named by the first message

- **WHEN** a session has no operator label and has received a first user message
- **THEN** its rail row shows a preview of that message instead of the session identifier

#### Scenario: placeholder falls back to the identifier

- **WHEN** a zero-turn placeholder session has no user message and no label
- **THEN** its rail row shows the short session identifier as the name

#### Scenario: long first message is truncated

- **WHEN** a session's display name exceeds the display cap
- **THEN** the row shows a truncated name ending with an ellipsis, and hovering the row reveals the full text via its title

#### Scenario: placeholder name updates after the first message

- **WHEN** the operator sends the first message into a placeholder session without a label
- **THEN** the row's name becomes the message preview without a page reload

#### Scenario: operator label takes precedence

- **WHEN** the operator sets a label on a session that already has messages
- **THEN** the rail row and session dialogs show the label (not the first-message preview) and keep it across turns and reloads

#### Scenario: renaming from the rail

- **WHEN** the operator picks rename in a rail session row's overflow menu and submits a new label
- **THEN** the row's name updates to the label without a page reload, and clearing the label falls back to the first-message preview

#### Scenario: rename dialog saves what it shows

- **WHEN** the operator types a non-empty name into the rail's rename dialog and confirms the save
- **THEN** the stored label equals the displayed text, and a subsequent page load shows the same name (a confirmed save is never a silent no-op)

#### Scenario: label writes through any path update the row live

- **WHEN** a label write is accepted through any path (rail dialog or the label API) while the session list is open in a browser
- **THEN** that session's rail row shows the new label driven by the session update event, without a manual reload

#### Scenario: restore preserves the row name

- **WHEN** an archived session that was named by its first-prompt preview (or carried an operator label) is restored to a project
- **THEN** the restored row shows the same name source as before archival instead of the raw session identifier
