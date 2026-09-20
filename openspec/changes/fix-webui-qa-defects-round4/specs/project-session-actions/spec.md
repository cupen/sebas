## MODIFIED Requirements

### Requirement: Session rows are named by the first prompt

A rail session row SHALL display a name for the session: if the operator has set a label for the session, the label SHALL be shown; otherwise the preview of the session's first user message SHALL be used. A session that has neither a label nor any user message (a zero-turn placeholder) SHALL fall back to its short session identifier. Names longer than the implementation-defined display cap SHALL be truncated with an ellipsis, while the row's hover title SHALL carry the full text. When the first message is sent to a placeholder session, its name SHALL update from the identifier to the message preview without a page reload — unless an operator label is set, which SHALL stay stable. The operator SHALL be able to set and change the label from the rail (row actions), and the rail's session dialogs SHALL name the session by the same label as its row.

Restoring an archived session SHALL preserve the naming inputs captured at archive time: the first-prompt preview and the operator label (if any) SHALL survive the restore, so the restored row is named exactly as before archival and never regresses to the raw session identifier while a naming source exists.

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

#### Scenario: restore preserves the row name

- **WHEN** an archived session that was named by its first-prompt preview (or carried an operator label) is restored to a project
- **THEN** the restored row shows the same name source as before archival instead of the raw session identifier
