## MODIFIED Requirements

### Requirement: Session archive

The rail session row's overflow menu SHALL be the operator-facing archive entry: archiving moves the session to the History group and carries the close semantics (the child is killed if active; the confirm dialog warns about pending submissions that will be discarded). The focused session's header SHALL render no action buttons and no navigation link. An archived session SHALL be read-only — the operator cannot send messages into it, cannot close it, and cannot switch to it as the active session. An archived session SHALL be restorable to its original project. The archive entry SHALL carry the session's identity — agent binding (`agent_kind`), desired permission mode, and the current/available model catalog — and restoring SHALL rebuild the session with that identity intact, so the restored session continues with the same agent and model surface it had before archiving. Legacy archive entries without identity fields SHALL restore with the existing fallbacks and the UI SHALL present that fallback honestly.

#### Scenario: archive a session

- **WHEN** the operator picks archive in a rail session row's overflow menu and confirms
- **THEN** the session is moved to the History group, marked as read-only, and cannot be interacted with

#### Scenario: archived session is read-only

- **WHEN** the operator attempts to send a message to an archived session
- **THEN** the system rejects the message with a 400 response stating the session is archived

#### Scenario: restore archived session

- **WHEN** the operator clicks an archived session in the History group
- **THEN** the session is restored to its original project, becomes writable, and is activated

#### Scenario: focused session header offers no actions

- **WHEN** a session is focused and the operator looks at the session header
- **THEN** the header shows display information only — no Archive button, no Close button, and no "All sessions" link

#### Scenario: 恢复保留 agent 身份与模型面

- **WHEN** a session bound to a non-default agent with a selected model is archived and then restored
- **THEN** the restored session reports the same `agent_kind`, desired mode, and current/available models, and the next turn runs on that agent

#### Scenario: 旧归档条目如实回退

- **WHEN** a legacy archive entry without identity fields is restored
- **THEN** the session rebuilds with the existing defaults and the header presents that fallback without fabricating an agent identity

### Requirement: Session rows are named by the first prompt

A rail session row SHALL display a name for the session: if the operator has set a label for the session, the label SHALL be shown; otherwise the preview of the session's first user message SHALL be used. A session that has neither a label nor any user message (a zero-turn placeholder) SHALL fall back to its short session identifier. Names longer than the implementation-defined display cap SHALL be truncated with an ellipsis, while the row's hover title SHALL carry the full text. When the first message is sent to a placeholder session, its name SHALL update from the identifier to the message preview without a page reload — unless an operator label is set, which SHALL stay stable. The operator SHALL be able to set and change the label from the rail (row actions), and the rail's session dialogs SHALL name the session by the same label as its row.

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
