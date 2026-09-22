# session-unread-badge Specification

## Purpose

Gives the operator a per-session unread count in the project rail so agent replies that arrive in unfocused sessions are visible at a glance, without opening each session. The count is computed server-side against an explicit chat-message definition and reconciled with a per-browser read cursor that is shared with the transcript's seen-boundary seam.

## Requirements

### Requirement: Chat message counting

For each session the system SHALL maintain a monotonic count of chat messages. A chat message is a transcript entry with operator-visible agent content (`element_type` `markdown` or `error`). Operator prompts, `thinking` entries, and `tool` entries SHALL NOT increment the count. The count SHALL be present in the session list projection and SHALL reach connected clients carried inside the session update event itself (the `session.updated` frame SHALL include the session's current message count) — clients SHALL be able to update the badge from the frame alone, with the rail's periodic refresh remaining only as a fallback. Count-bearing updates SHALL arrive without a manual reload.

The `session.updated` frame SHALL be the single source of truth for lifecycle semantics on the wire: its shape SHALL be `{ session_id, status_slug, turn_engaged, msg_count, pending }` with every key present on every frame — `status_slug` SHALL be one of `starting | queued | working | done | failed | waiting | dormant` (the projection of `(MappingState, card phase, parked approvals)` into the operator-facing word), `turn_engaged` SHALL be a boolean present on every frame (no "only present when true" rule), `msg_count` SHALL be present on every frame, and `pending` SHALL be the current queue of pending submissions in delivery order. The legacy `status` string field is removed entirely; no backwards-compatibility key is kept, because the core, webui, and frontend ship in the same binary and the wire protocol carries no independent version. The backend emits such a frame on every FSM phase flip — entering Spawning, WORKING start, Finished→DONE, terminal Error→FAILED, parked-approval entry/exit, stall watchdog force-settle — so the wire carries an explicit lifecycle event at every boundary; the rail, the composer submit affordance, and any other surface SHALL consume these flips in real time rather than poll the detail endpoint or fall back to `status_slug === 'working'` string heuristics. The `session.created` frame SHALL carry the same complete field set as `session.updated` (initially `status_slug: "starting"`, `turn_engaged: true`, `msg_count: 0`, `pending: []`).

#### Scenario: an agent reply increments the count

- **WHEN** an agent completes a visible reply segment in a session
- **THEN** that session's message count increases by the number of visible reply segments produced, and connected clients observe the new value from the session update frame

#### Scenario: process noise does not increment the count

- **WHEN** a session produces thinking entries, tool calls, or tool results
- **THEN** the session's message count does not change

#### Scenario: the operator's own prompt does not increment the count

- **WHEN** the operator sends a message into a session
- **THEN** the session's message count does not change due to that prompt

#### Scenario: the frame carries the count

- **WHEN** a session's message count changes and a browser is connected
- **THEN** the next session update frame for that session carries the new count value, and the browser's rail badge reflects it without any additional fetch

#### Scenario: the frame carries the live turn fact

- **WHEN** a session enters or leaves a live turn, spawning window, or parked approval state while a browser is connected
- **THEN** the next session update frame for that session carries `turn_engaged: false` for the unoccupied case and `true` otherwise (the key is present on every frame — there is no "only present when true" rule), and the composer's submit affordance matches the engine truth without waiting for a detail refetch

#### Scenario: every lifecycle flip emits a frame

- **WHEN** a session transitions across any lifecycle boundary — Spawning inserted, first agent event flips the card to WORKING, Finished flips to DONE, terminal Error flips to FAILED, a parked approval appears (Waiting), or the stall watchdog force-settles
- **THEN** the corresponding `session.updated` frame for that transition carries the new `status_slug` at the moment it flips, and the browser observes the boundary as a discrete event, not as the eventual result of the next HTTP poll

### Requirement: Unread badge on session rows

A rail session row SHALL display a highlighted number equal to the difference between the session's current message count and the browser's stored read anchor for that session, whenever that difference is greater than zero. Rows with no unread messages SHALL NOT display a badge. A row with unread messages SHALL be visually prominent at a glance: the badge numeral SHALL use a high-contrast emphasis treatment and the row SHALL carry an additional emphasis (such as a background tint) that distinguishes it from read rows before the operator fixates on the numeral. Focusing a session SHALL clear its badge (the read anchor advances to the current count). The read anchor SHALL be a single per-session segment count shared by every unread surface; the transcript's seen-boundary SHALL advance the same anchor when the operator reads to the boundary. The waiting badge for parked approvals SHALL remain independent of the unread badge. While live output streams into a focused session, the badge SHALL follow the read-anchor semantics of the `live-turn-stream` capability: anchored-and-at-bottom arrivals advance the anchor as they render instead of flashing the badge.

The read anchor SHALL be established from the session's empty state when a
freshly created placeholder session is focused, so the first focused exchange
— including the placeholder-to-spawn transition and the reply it produces —
never flashes the badge or the shared unseen-turn seam. Arrivals into a
focused session with the document hidden (background tab) SHALL NOT advance
the anchor: they remain unseen and are flagged when the operator returns.

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

#### Scenario: no badge without unread messages

- **WHEN** a session's message count equals its stored read anchor
- **THEN** its rail row shows no unread badge

#### Scenario: first focused exchange of a fresh placeholder

- **WHEN** the operator creates a placeholder session, sends the first message, and watches the spawned child's reply while staying focused at the live edge
- **THEN** no unread badge appears on the session's rail row and no unseen-turn seam is drawn for that exchange

#### Scenario: hidden-tab arrivals stay unseen

- **WHEN** the focused session receives reply segments while the page is in a hidden background tab
- **THEN** the read anchor does not advance and the content is reported as unseen when the operator returns

### Requirement: Unread cursor is per-browser and shared with the seen boundary

The read anchor SHALL be stored per browser (localStorage), keyed by session, and SHALL NOT be recorded server-side. The anchor SHALL be the segment count: every surface that marks content as read — focusing the session, streaming-while-reading, and reading the transcript down to the seen boundary — SHALL advance the same stored segment count, so the unread badge and the transcript's seen-boundary never disagree about what has been read. A session with no stored anchor SHALL be treated as fully read, so history never surfaces as unread after a cache clear or a first visit from a new browser.

#### Scenario: first visit shows no unread

- **WHEN** the browser has no stored read anchor for a session with existing messages
- **THEN** the session shows no unread badge

#### Scenario: seam and badge share the anchor

- **WHEN** the operator reads a session up to its seen boundary and returns to the rail
- **THEN** the unread badge and the transcript seam agree: reading to the bottom clears the badge, and new messages below the seam produce both the seam and a badge

#### Scenario: anchors are independent per browser

- **WHEN** the same sessions are viewed from a different browser
- **THEN** that browser's own anchors apply and the server holds no record of either
