## MODIFIED Requirements

### Requirement: New session without prompt

The workbench SHALL support creating a 0-turn placeholder session without requiring a prompt. The placeholder SHALL appear in the session list immediately and SHALL be activated. An ACP child SHALL NOT block creation: focusing a placeholder session that has no live child SHALL start the child in the background — resuming the recorded conversation when the session mapping allows it — and the first message SHALL also start the child if focus never did. The project row SHALL have a dedicated "New session" button. A failed background start SHALL NOT remove the placeholder.

#### Scenario: create empty session from project

- **WHEN** the operator clicks the `+` button on a project row
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

The rail session row's overflow menu SHALL be the operator-facing archive entry: archiving moves the session to the History group and carries the close semantics (the child is killed if active; the confirm dialog warns about pending submissions that will be discarded). The focused session's header SHALL render no action buttons and no navigation link. An archived session SHALL be read-only — the operator cannot send messages into it, cannot close it, and cannot switch to it as the active session. An archived session SHALL be restorable to its original project.

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
