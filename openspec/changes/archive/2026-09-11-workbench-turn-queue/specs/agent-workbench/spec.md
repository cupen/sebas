## ADDED Requirements

### Requirement: Pending submissions stack above the composer

The workbench SHALL render the focused session's pending submissions directly above the composer input, in delivery order, without occupying transcript space. Each entry SHALL state its disposition: submissions staged during a spawn SHALL read as combining into the session's first message, and submissions queued behind a running turn SHALL read as waiting, with their position. A submission SHALL NOT appear in the transcript until it starts (or is combined at activation); at that moment it SHALL leave the stack and the transcript SHALL show it as a submission entry.

The stack SHALL support removal of any entry and drag-reordering within the entry's own disposition group. Entries carrying priority (`/btw`) SHALL be rendered as priority and SHALL NOT be draggable, and no drag SHALL place a non-priority entry ahead of a priority one. Submitting while entries are pending SHALL append to the stack — it SHALL NOT replace, discard, or silently merge into an existing entry's text.

When the session ends or is closed while entries are pending, those entries SHALL be reported as not executed in a single explicit notice naming them (the stack itself disappears with the session, so the notice is the record); the stack SHALL never shrink without an explanation.

#### Scenario: stack renders pending submissions in order

- **WHEN** the focused session has submissions staged during spawn and submissions queued behind a running turn
- **THEN** the region above the composer lists them in delivery order, distinguishing "combining into the first message" from "waiting" with position

#### Scenario: submitting appends instead of replacing

- **WHEN** the operator submits a message while entries are already pending
- **THEN** the new submission appears as an additional stack entry and no existing entry's text is altered

#### Scenario: remove and reorder take effect

- **WHEN** the operator removes a pending entry, or drags a non-priority entry to a new position in its group
- **THEN** the stack reflects the change immediately and still reflects it after the next refresh

#### Scenario: priority entries are pinned

- **WHEN** the stack contains a `/btw` priority entry
- **THEN** it is rendered as priority, cannot be dragged, and no drag can place another entry ahead of it

#### Scenario: a started submission leaves the stack and enters the transcript

- **WHEN** a pending submission starts its turn (or is combined at activation)
- **THEN** it is removed from the stack and appears in the transcript as a submission entry, so the conversation shows what is actually running

#### Scenario: pending entries are not silently dropped at session end

- **WHEN** the focused session fails or is closed while entries are pending
- **THEN** a notice names those entries as not executed, and the stack clears only together with that notice

## MODIFIED Requirements

### Requirement: Rail session close entry

Each session row in the workbench rail (project groups and Inbox) SHALL expose a close action alongside the existing archive button, with the same semantics as `POST /api/sessions/{key}/close` (kill the child when active, drop the mapping). Closing SHALL require no confirmation for inactive (dormant/done/failed) sessions and SHALL require an inline confirmation for active (starting/queued/working) sessions. When the session being closed has pending submissions, the confirmation SHALL state how many will be discarded and never executed. When the closed session was the focused one, the workbench SHALL return to the no-focus empty state.

#### Scenario: close a dormant session from the rail

- **WHEN** the operator clicks the close button on a dormant session row
- **THEN** the session is closed immediately, the row disappears, and no confirmation dialog is shown

#### Scenario: closing a working session asks first

- **WHEN** the operator clicks the close button on a session whose child is still running
- **THEN** an inline confirmation is shown first, and only on confirm is the close request sent

#### Scenario: closing with pending submissions names the loss

- **WHEN** the operator clicks the close button on a session that has pending submissions
- **THEN** the confirmation states how many pending submissions will be discarded, and closing removes them without delivering them to any later session

#### Scenario: closing the focused session clears the stage

- **WHEN** the operator closes the session that is currently focused
- **THEN** the workbench stage returns to the no-focus empty state and the composer re-enters creation mode
