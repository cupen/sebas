## MODIFIED Requirements

### Requirement: Unread badge on session rows

A rail session row SHALL display a highlighted number equal to the difference between the session's current message count and the browser's stored read anchor for that session, whenever that difference is greater than zero. Rows with no unread messages SHALL NOT display a badge. Focusing a session SHALL clear its badge (the read anchor advances to the current count). The waiting badge for parked approvals SHALL remain independent of the unread badge. While live output streams into a focused session, the badge SHALL follow the read-anchor semantics of the `live-turn-stream` capability: anchored-and-at-bottom arrivals advance the anchor as they render instead of flashing the badge.

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
