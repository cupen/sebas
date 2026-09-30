## MODIFIED Requirements


### Requirement: Session rows are named by the first prompt

A rail session row SHALL display a name for the session: if the operator has set a label for the session, the label SHALL be shown; otherwise, if an auto-generated title exists, the title SHALL be shown; otherwise the preview of the session's first user message SHALL be used. A session that has neither a label, nor an auto-generated title, nor any user message (a zero-turn placeholder) SHALL show the readable placeholder name 「未命名会话」 instead of a raw (or truncated) session identifier. Names longer than the implementation-defined display cap SHALL be truncated with an ellipsis, while the row's hover title SHALL carry the full text. When the first message is sent to a placeholder session, its name SHALL update from the 「未命名会话」 placeholder to the message preview without a page reload — unless an operator label is set, which SHALL stay stable. The operator SHALL be able to set and change the label from the rail (row actions), and the rail's session dialogs SHALL name the session by the same label as its row.

The first-message preview SHALL be captured once, at the session's first user message, and SHALL remain the naming source across subsequent turn events — including child-process crash and respawn, non-terminal errors, and cancel — for as long as no operator label or auto-generated title exists. A session row whose displayed name is neither its first-message preview, nor its label, nor its auto-title, nor the 「未命名会话」 placeholder is a violation.

#### Scenario: named by the first message

- **WHEN** a session has no operator label, no auto-generated title, and has received a first user message
- **THEN** its rail row shows a preview of that message instead of the session identifier

#### Scenario: placeholder falls back to the identifier

- **WHEN** a zero-turn placeholder session has no user message, no label, and no auto-generated title
- **THEN** its rail row shows 「未命名会话」 as the name, never a raw or truncated session identifier

#### Scenario: long first message is truncated

- **WHEN** a session's display name exceeds the display cap
- **THEN** the row shows a truncated name ending with an ellipsis, and hovering the row reveals the full text via its title

#### Scenario: placeholder name updates after the first message

- **WHEN** the operator sends the first message into a placeholder session without a label
- **THEN** the row's name becomes the message preview without a page reload

#### Scenario: preview stays anchored to the first message

- **WHEN** a session without label or auto-title receives a second, different user message
- **THEN** the row name and the focused-session header keep showing a preview of the FIRST user message — they SHALL NOT drift to the latest message

#### Scenario: operator label takes precedence

- **WHEN** the operator sets a label on a session that already has messages
- **THEN** the rail row and session dialogs show the label (not the first-message preview) and keep it across turns and reloads

#### Scenario: auto-titled row shows the title

- **WHEN** a session has no operator label and its auto-generated title has arrived
- **THEN** the rail row and the focused-session header show the title (not the preview or the identifier), updated live without a page reload

#### Scenario: turn-produced naming updates live

- **WHEN** a turn produces a naming change (first preview appears or an auto-title arrives) while the rail is open
- **THEN** the affected row (and the focused header, if applicable) re-renders the new name without a page reload or list refetch

#### Scenario: renaming from the rail

- **WHEN** the operator picks rename in a rail session row's overflow menu and submits a new label
- **THEN** the row's name updates to the label without a page reload, and clearing the label falls back to the first-message preview

#### Scenario: rename dialog saves what it shows

- **WHEN** the operator types a non-empty name into the rail's rename dialog and confirms the save
- **THEN** the stored label equals the displayed text, and a subsequent page load shows the same name (a confirmed save is never a silent no-op)

#### Scenario: header follows the rename immediately

- **WHEN** the operator renames the focused session
- **THEN** the focused-session header shows the new label without a page reload

#### Scenario: label writes through any path update the row live

- **WHEN** a label write is accepted through any path (rail dialog or the label API) while the session list is open in a browser
- **THEN** that session's rail row shows the new label driven by the session update event, without a manual reload

#### Scenario: restore preserves the row name

- **WHEN** an archived session that was named by its first-prompt preview (or carried an operator label) is restored to a project
- **THEN** the restored row shows the same name source as before archival instead of the raw session identifier

#### Scenario: crash 重生后行名仍取首条 prompt

- **WHEN** a session named by its first prompt ("hello") experiences a mid-turn child crash and the next message respawns the child
- **THEN** the rail row and focused-session header still display the first-prompt preview, not any later message text

#### Scenario: 未发消息的占位会话行名可读

- **WHEN** a session is created through the creation dialog and has no messages yet
- **THEN** its rail row shows a readable placeholder name (「未命名会话」) instead of a truncated raw identifier, and updates to the first-prompt preview when the first message lands
