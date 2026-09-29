# session-unread-badge Specification

## Purpose

Gives the operator a per-session unread count in the project rail so agent replies that arrive in unfocused sessions are visible at a glance, without opening each session. The count is computed server-side against an explicit chat-message definition and reconciled with a per-browser read cursor that is shared with the transcript's seen-boundary seam.

## Requirements

### Requirement: Chat message counting

For each session the system SHALL maintain a monotonic count of chat messages. A chat message is a run of operator-visible agent content in the transcript: contiguous `markdown` entries of one agent reply fold into ONE message, an `error` entry counts as one message on its own, and empty entries neither count nor interrupt a run. Operator prompts, `thinking` entries, and `tool` entries SHALL NOT increment the count. The count SHALL be present in the session list projection and SHALL reach connected clients carried inside the session update event itself (the `session.updated` frame SHALL include the session's current message count) — clients SHALL be able to update the badge from the frame alone, with the rail's periodic refresh remaining only as a fallback. Count-bearing updates SHALL arrive without a manual reload. (Note: the badge counts reply segments, while the unseen-turn seam counts turns — the two numbers are different units by design and need not be equal.)

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

A rail session row SHALL display a highlighted number equal to the difference between the session's current message count and the browser's stored read anchor for that session, whenever that difference is greater than zero AND the session is not the focused session (or the document is hidden). Rows with no unread messages SHALL NOT display a badge. A row with unread messages SHALL be visually prominent at a glance: the badge numeral SHALL use a high-contrast emphasis treatment and the row SHALL carry an additional emphasis (such as a background tint) that distinguishes it from read rows before the operator fixates on the numeral. Focusing a session SHALL clear its badge (the read anchor advances to the current count). The read anchor SHALL be a single per-session segment count shared by every unread surface; the transcript's seen-boundary SHALL advance the same anchor when the operator reads to the boundary. The waiting badge for parked approvals SHALL remain independent of the unread badge. While live output streams into a focused session, the badge SHALL follow the read-anchor semantics of the `live-turn-stream` capability: anchored-and-at-bottom arrivals advance the anchor as they render instead of flashing the badge.

The read anchor SHALL be established from the session's empty state when a
freshly created placeholder session is focused, so the first focused exchange
— including the placeholder-to-spawn transition and the reply it produces —
never flashes the badge or the shared unseen-turn seam. Arrivals into a
focused session with the document hidden (background tab) SHALL NOT advance
the anchor: they remain unseen and are flagged when the operator returns.

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

### Requirement: Unseen-turn seam presence follows arrival context

未读分界线（unseen-turn seam）SHALL 只随到达上下文出现：对会话非聚焦（或页面隐藏）期间到达的可见回复，操作员下次进入该会话时 SHALL 恰好绘出一次分界线；对聚焦且贴底观看期间流式到达的内容 SHALL NOT 绘出；分界线随已读即清——边界内容一旦被看过，后续进入同一会话不得再对已看内容重现分界线。

#### Scenario: refocus after unfocused completion shows the seam

- **WHEN** 会话在非聚焦状态下完成一段可见回复，操作员随后聚焦该会话
- **THEN** 侧栏行先出现未读徽标，转写在未读内容之上绘出分界线

#### Scenario: focused completion draws no seam

- **WHEN** 操作员正聚焦会话并停留在直播边缘时回复完成
- **THEN** 该段内容不绘出分界线，也不误标未读

#### Scenario: seam clears after being read

- **WHEN** 操作员已看过分界线以下内容后离开再回到该会话
- **THEN** 已看内容之上不再重现分界线

### Requirement: Focus anchors at the server's current count

聚焦会话推进读锚 SHALL 以服务端当前段计数为准：即使聚焦动作与回合定稿竞速，锚推进路径不得停在回合前的旧值（establishment 对已有锚只增不减的规则不变，但「已有锚 + 新段落」的推进不得依赖竞速窗口内的陈旧本地状态）。

#### Scenario: immediate refocus after a completed turn parks the full count

- **WHEN** 会话完成第二回合（服务端 msg_count=2）且操作员随即聚焦该会话
- **THEN** 存储读锚推进到 2（不是回合前的 1），徽标不再复燃
