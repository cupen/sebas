## ADDED Requirements

### Requirement: History group expanded state persists

The History group's expanded/collapsed state SHALL persist locally in the
browser alongside the persisted pane dimensions and SHALL be restored on the
next load without server round-trips, so a deliberately expanded history does
not snap back to collapsed on reload.

#### Scenario: expanded history survives a reload

- **WHEN** the operator expands the History group and reloads the page
- **THEN** the History group renders expanded with its persisted count

#### Scenario: collapsed history survives a reload

- **WHEN** the operator collapses the History group and reloads the page
- **THEN** the History group renders collapsed

### Requirement: Rail rows surface lifecycle state

Each session row in the project rail SHALL present the session's lifecycle
state as projected by the session update frame's `status_slug`
(`starting | queued | working | done | failed | waiting | dormant`), so the
operator can tell an in-flight session from a dormant or failed one at a
glance without opening it. The presentation SHALL update in real time as
`session.updated` frames arrive, including the force-settle flip emitted by
the stall watchdog, and SHALL NOT keep showing a stale state word or pending
count after the frame that ended it.

#### Scenario: working session is distinguishable from a dormant one

- **WHEN** a session's turn is running while a sibling session sits dormant
- **THEN** the rail rows present different lifecycle states for the two
  sessions (e.g. 进行中 vs 休眠), driven by their latest frames

#### Scenario: force-settle clears the waiting presentation

- **WHEN** a turn parked on approvals is force-settled by the stall watchdog
- **THEN** the row's waiting presentation (state word and pending count) is
  replaced by the settled state carried by the corresponding
  `session.updated` frame, with no stale 等待 marker remaining

### Requirement: Creation dialog accepts an optional session title

The rail's creation dialog SHALL accept an optional session title field.
When provided, the placeholder session SHALL be created under that title and
the rail SHALL show it immediately; when left empty, the current naming
behavior applies unchanged. The title SHALL be editable afterwards exactly
like any renamed session.

#### Scenario: titled creation

- **WHEN** the operator types a title into the creation dialog and creates
  the session
- **THEN** the placeholder appears in the rail under that title without a
  separate rename step

#### Scenario: untitled creation keeps current behavior

- **WHEN** the operator leaves the title empty and creates the session
- **THEN** the session is named exactly as before this requirement existed
