## MODIFIED Requirements

### Requirement: Session archive

The rail session row's overflow menu SHALL be the operator-facing archive entry: archiving moves the session to the History group and carries the close semantics (the child is killed if active; the confirm dialog warns about pending submissions that will be discarded). The focused session's header SHALL render no action buttons and no navigation link. An archived session SHALL be read-only — the operator cannot send messages into it, cannot close it, and cannot switch to it as the active session. An archived session SHALL be restorable to its original project.

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

#### Scenario: restore into an unregistered project is not silent

- **WHEN** the operator restores a session whose original project path is not a registered project
- **THEN** the outcome notification states the project path it was restored to, and no archived entry disappears without a stated outcome

#### Scenario: focused session header offers no actions

- **WHEN** a session is focused and the operator looks at the session header
- **THEN** the header shows display information only — no Archive button, no Close button, and no "All sessions" link
