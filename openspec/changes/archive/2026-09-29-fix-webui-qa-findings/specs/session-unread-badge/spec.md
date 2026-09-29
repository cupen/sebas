## MODIFIED Requirements

### Requirement: Unread badge on session rows

A rail session row SHALL display a highlighted number equal to the difference between the session's current message count and the browser's stored read anchor for that session, whenever that difference is greater than zero AND the session is not the focused session (or the document is hidden). Rows with no unread messages SHALL NOT display a badge. A row with unread messages SHALL be visually prominent at a glance: the badge numeral SHALL use a high-contrast emphasis treatment and the row SHALL carry an additional emphasis (such as a background tint) that distinguishes it from read rows before the operator fixates on the numeral. Focusing a session SHALL clear its badge (the read anchor advances to the current count). The read anchor SHALL be a single per-session segment count shared by every unread surface; the transcript's seen-boundary SHALL advance the same anchor when the operator reads to the boundary. The waiting badge for parked approvals SHALL remain independent of the unread badge. While live output streams into a focused session, the badge SHALL follow the read-anchor semantics of the `live-turn-stream` capability: anchored-and-at-bottom arrivals advance the anchor as they render instead of flashing the badge.

The read anchor SHALL be established from the session's empty state when a freshly created placeholder session is focused, so the first focused exchange — including the placeholder-to-spawn transition and the reply it produces — never flashes the badge or the shared unseen-turn seam. Arrivals into a focused session with the document hidden (background tab) SHALL NOT advance the anchor: they remain unseen and are flagged when the operator returns.

The badge computation SHALL be driven by the live session-update stream: when a non-focused session's message count advances, its rail row SHALL re-render the badge without a page reload and without requiring the session list to be refetched. A regression that leaves the row markup capable of rendering the badge but never computes a non-zero unread for arriving replies SHALL be treated as a violation of this requirement — the badge SHALL be exercised by an automated browser-level regression test covering the non-focused-arrival path.

Returning to a session that has unseen segments SHALL surface the transcript's unread seam at the boundary between seen and unseen content (the same anchor that powers the badge), so the operator can see where their reading left off.

#### Scenario: new reply on an unfocused session

- **WHEN** a session the operator is not focused on receives visible reply segments
- **THEN** its rail row shows the highlighted unread number, incremented without a page reload

#### Scenario: unread rows are prominent

- **WHEN** the rail lists a session with unread messages next to sessions without
- **THEN** the unread row is distinguishable at a glance by its emphasis treatment before reading the numeral

#### Scenario: focusing the session clears the badge

- **WHEN** the operator focuses a session that has an unread badge
- **THEN** the badge disappears and the stored read anchor equals the current message count

#### Scenario: streamed arrival while focused at the bottom

- **WHEN** visible reply segments stream into the focused session while the operator is scrolled to the bottom
- **THEN** the badge does not appear and the stored read anchor advances with the streamed content

#### Scenario: first focused exchange of a fresh placeholder

- **WHEN** the operator creates a placeholder session, sends the first message, and watches the spawned child's reply while staying focused at the live edge
- **THEN** no unread badge appears on the session's rail row and no unseen-turn seam is drawn for that exchange

#### Scenario: hidden-tab arrivals stay unseen

- **WHEN** the focused session receives reply segments while the page is in a hidden background tab
- **THEN** the read anchor does not advance and the content is reported as unseen when the operator returns

#### Scenario: no badge without unread messages

- **WHEN** a session's message count equals its stored read anchor
- **THEN** its rail row shows no unread badge

#### Scenario: badge survives turn completion of a background session

- **WHEN** a background session's turn completes (status reaches its terminal slug) while the operator is focused on another session
- **THEN** the background session's rail row shows the unread badge for the segments that arrived during the turn, without any reload or list refetch

#### Scenario: returning to the session shows the unread seam

- **WHEN** the operator focuses a session that accumulated unseen segments while unfocused
- **THEN** the transcript renders the unread seam at the seen/unseen boundary before the newly arrived content
