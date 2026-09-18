## MODIFIED Requirements

### Requirement: Unread badge on session rows

A rail session row SHALL display a highlighted number equal to the difference between the session's current message count and the browser's stored read anchor for that session, whenever that difference is greater than zero. Rows with no unread messages SHALL NOT display a badge. Focusing a session SHALL clear its badge (the read anchor advances to the current count). The waiting badge for parked approvals SHALL remain independent of the unread badge. While live output streams into a focused session, the badge SHALL follow the read-anchor semantics of the `live-turn-stream` capability: anchored-and-at-bottom arrivals advance the anchor as they render instead of flashing the badge.

The read anchor SHALL be established from the session's empty state when a
freshly created placeholder session is focused, so the first focused exchange
— including the placeholder-to-spawn transition and the reply it produces —
never flashes the badge or the shared unseen-turn seam. Arrivals into a
focused session with the document hidden (background tab) SHALL NOT advance
the anchor: they remain unseen and are flagged when the operator returns.

#### Scenario: new reply on an unfocused session

- **WHEN** a session the operator is not focused on receives visible reply segments
- **THEN** its rail row shows the highlighted unread number, incremented without a page reload

#### Scenario: focusing the session clears the badge

- **WHEN** the operator focuses a session that has an unread badge
- **THEN** the badge disappears and the stored read anchor equals the current message count

#### Scenario: no badge without unread messages

- **WHEN** a session's message count equals its stored read anchor
- **THEN** its rail row shows no unread badge

#### Scenario: streamed arrival while focused at the bottom

- **WHEN** visible reply segments stream into the focused session while the operator is scrolled to the bottom
- **THEN** the badge does not appear and the stored read anchor advances with the streamed content

#### Scenario: first focused exchange of a fresh placeholder

- **WHEN** the operator creates a placeholder session, sends the first message, and watches the spawned child's reply while staying focused at the live edge
- **THEN** no unread badge appears on the session's rail row and no unseen-turn seam is drawn for that exchange

#### Scenario: hidden-tab arrivals stay unseen

- **WHEN** the focused session receives reply segments while the page is in a hidden background tab
- **THEN** the read anchor does not advance and the content is reported as unseen when the operator returns
