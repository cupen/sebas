## ADDED Requirements

### Requirement: Neutral presentation content contract

The IM service's neutral outbound presentation SHALL maintain exactly one presentation instance per session per turn, seeded when the turn starts and reset at each new user turn; each user turn SHALL produce a fresh presentation that threads to that turn's input, while presentations from earlier turns SHALL remain frozen (no further updates). Streaming content deltas SHALL accumulate in an in-memory body and flush as a single coalesced update per debounce tick; a terminal event (turn finished, terminal error) SHALL flush immediately, bypassing the debounce. When a turn finishes, the presentation SHALL enter a terminal state marking completion; a terminal error SHALL append an error marker and finalize the presentation as the record of the failure. Thinking content SHALL be either folded into a collapsed panel (when shown) or dropped entirely (when hidden), per the channel's rendering policy. The presentation body SHALL enforce a content budget (characters and element count); when a flush would exceed it, the oldest elements SHALL be evicted, and when a turn's content reaches a high fraction of the budget the IM service SHALL rotate — finalize the current presentation, seed a new one carrying a continuation note, and continue streaming into it. The IM service SHALL drop the presentation state when a session ends (terminal error, channel close, or explicit close) so subsequent interaction cannot update a stale presentation; message-id mappings SHALL be overwritten per turn.

#### Scenario: one presentation per turn, earlier frozen

- **WHEN** a session turn streams content and the user then sends a second message
- **THEN** the adapter emits a fresh presentation for the second turn and the first turn's presentation receives no further updates

#### Scenario: in-flight message does not spawn a second presentation

- **WHEN** a user message arrives while a turn is still streaming
- **THEN** the message is queued and no additional presentation instance is created for it

#### Scenario: deltas coalesce into one flush

- **WHEN** multiple content deltas for one turn arrive within a debounce window on any IM channel
- **THEN** the IM frontend flushes them as a single coalesced presentation update

#### Scenario: terminal event flushes immediately

- **WHEN** a turn-finished or terminal-error event arrives mid-debounce-window
- **THEN** the pending body flushes without waiting for the window to expire

#### Scenario: presentation finalized on terminal error

- **WHEN** the session reports a terminal error mid-turn
- **THEN** the presentation body ends with an error marker and receives no further updates

#### Scenario: thinking shown folded or hidden

- **WHEN** the channel's rendering policy is show and thinking deltas stream in
- **THEN** the body contains a collapsed panel holding the thinking content, separate from the output
- **AND** when the policy is hide, no thinking content appears in the body

#### Scenario: budget eviction and rotation

- **WHEN** appending a new element would push the presentation past its element budget, or a turn's content reaches the rotation threshold
- **THEN** the oldest element is evicted so the flush fits the budget, and on rotation the current presentation is finalized and a continuation presentation carries a note

#### Scenario: presentation state dropped on session end

- **WHEN** a session terminates with a terminal error, channel close, or explicit close
- **THEN** the presentation state is removed and later updates for that session are discarded
