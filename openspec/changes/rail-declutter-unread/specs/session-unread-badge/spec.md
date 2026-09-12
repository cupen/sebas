## Purpose

Gives the operator a per-session unread count in the project rail so agent replies that arrive in unfocused sessions are visible at a glance, without opening each session. The count is computed server-side against an explicit chat-message definition and reconciled with a per-browser read cursor that is shared with the transcript's seen-boundary seam.

## ADDED Requirements

### Requirement: Chat message counting

For each session the system SHALL maintain a monotonic count of chat messages. A chat message is a transcript entry with operator-visible agent content (`element_type` `markdown` or `error`). Operator prompts, `thinking` entries, and `tool` entries SHALL NOT increment the count. The count SHALL be present in the session list projection and SHALL reach connected clients when it changes — carried by session update events, with the rail's periodic refresh as a fallback — so the badge updates without a manual reload.

#### Scenario: an agent reply increments the count

- **WHEN** an agent completes a visible reply segment in a session
- **THEN** that session's message count increases by the number of visible reply segments produced, and connected clients observe the new value

#### Scenario: process noise does not increment the count

- **WHEN** a session produces thinking entries, tool calls, or tool results
- **THEN** the session's message count does not change

#### Scenario: the operator's own prompt does not increment the count

- **WHEN** the operator sends a message into a session
- **THEN** the session's message count does not change due to that prompt

### Requirement: Unread badge on session rows

A rail session row SHALL display a highlighted number equal to the difference between the session's current message count and the browser's stored read anchor for that session, whenever that difference is greater than zero. Rows with no unread messages SHALL NOT display a badge. Focusing a session SHALL clear its badge (the read anchor advances to the current count). The waiting badge for parked approvals SHALL remain independent of the unread badge.

#### Scenario: new reply on an unfocused session

- **WHEN** a session the operator is not focused on receives visible reply segments
- **THEN** its rail row shows the highlighted unread number, incremented without a page reload

#### Scenario: focusing the session clears the badge

- **WHEN** the operator focuses a session that has an unread badge
- **THEN** the badge disappears and the stored read anchor equals the current message count

#### Scenario: no badge without unread messages

- **WHEN** a session's message count equals its stored read anchor
- **THEN** its rail row shows no unread badge

### Requirement: Unread cursor is per-browser and shared with the seen boundary

The read anchor SHALL be stored per browser (localStorage), keyed by session, and SHALL NOT be recorded server-side. The unread badge and the transcript's seen-boundary seam SHALL share the same anchor so they never disagree about what has been read. A session with no stored anchor SHALL be treated as fully read, so history never surfaces as unread after a cache clear or a first visit from a new browser.

#### Scenario: first visit shows no unread

- **WHEN** the browser has no stored read anchor for a session with existing messages
- **THEN** the session shows no unread badge

#### Scenario: seam and badge share the anchor

- **WHEN** the operator reads a session up to its seen boundary and returns to the rail
- **THEN** the unread badge and the transcript seam agree: reading to the bottom clears the badge, and new messages below the seam produce both the seam and a badge

#### Scenario: anchors are independent per browser

- **WHEN** the same sessions are viewed from a different browser
- **THEN** that browser's own anchors apply and the server holds no record of either
