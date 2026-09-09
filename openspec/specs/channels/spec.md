# channels Specification

## Purpose

Defines the core's neutral channel abstraction: a channel-agnostic session key, inbound event model, outbound presentation model, and adapter registry. The core depends only on this abstraction; concrete channels (Feishu, WebUI, future IM/agent clients) plug in as adapters without reshaping core domain types. Terminology (session, project, workbench, channel, adapter, execution body) is defined in `openspec/glossary.md`.

## Requirements

### Requirement: Neutral channel key

The core SHALL identify every session with a channel-neutral `ChannelKey` carrying a channel name and a channel-specific opaque reference. The core SHALL NOT treat any concrete channel's id shape (e.g. Feishu `chat_id`/`thread_id`) as a first-class domain concept. A session's originating channel SHALL be recorded so that outbound events are routed back to the same channel.

#### Scenario: sessions are addressed by channel key

- **WHEN** a session is created by an inbound message from channel `feishu` with key `oc_x`
- **THEN** the core addresses that session by a `ChannelKey` combining `feishu` and `oc_x`
- **AND** outbound events for the session are routed to the adapter registered for `feishu`

#### Scenario: two channels share one core

- **WHEN** a `web` session and a separate `feishu` session exist
- **THEN** both are visible in the same snapshot, each addressed by its own channel key, and neither channel's adapter receives the other's outbound events

### Requirement: Neutral inbound events

The core SHALL accept inbound session interactions in a channel-neutral model: text, media, button callback, and form callback, each carrying its originating `ChannelKey`. The IM service's frontend logic SHALL dispatch on the neutral event model only and SHALL NOT depend on any concrete channel's event type; what reaches the core across the session channel SHALL be distilled session requests (message, decision, state mutation), never a concrete channel's event shape.

#### Scenario: text from any channel routes identically

- **WHEN** a text event arrives with channel key `feishu:oc_x`
- **THEN** the IM frontend treats it as the same text event as one with channel key `web:w1`, differing only in the key
- **AND** the channel-specific reply target (message id / thread root) is carried as channel-neutral metadata, not as a first-class field

#### Scenario: button callback maps to channel action

- **WHEN** a button callback arrives from channel `feishu`
- **THEN** the IM frontend dispatches the neutral button action and resolves the concrete Feishu callback reference back to the session, issuing the corresponding session-channel request

### Requirement: Neutral outbound presentation

The system SHALL keep a channel-neutral outbound presentation model — the channel-agnostic card model: a per-turn presentation instance that streams updates, freezes at turn end, and carries interactive elements (buttons, forms, selects) whose actions resolve to neutral events. The IM service SHALL build and maintain this presentation from the core's session content (session events and turn entries); the core SHALL NOT hold any IM-facing presentation state. Concrete channels SHALL render this model into their native presentation (e.g. Feishu card schema 2.0 JSON).

#### Scenario: streaming presentation coalesces

- **WHEN** multiple content deltas for one turn arrive within a debounce window on any IM channel
- **THEN** the IM frontend flushes them as a single coalesced presentation update for that channel

#### Scenario: interactive element resolves to neutral action

- **WHEN** a user activates an interactive element on a rendered presentation (e.g. clicks a permission button)
- **THEN** the channel adapter translates it into a neutral button-callback event addressed to the originating session, and the IM frontend issues the corresponding session-channel request

### Requirement: Adapter registry

The IM service SHALL maintain an adapter registry mapping channel names to their adapters; the core process SHALL NOT host IM adapters. A channel's adapter SHALL be active only when its configuration enables it; the registry SHALL be queryable for which channels are active and their health. Registering a new channel SHALL NOT require changing core session routing or domain types.

#### Scenario: disabled channel is absent from registry

- **WHEN** a channel's configuration disables it and provides no credentials
- **THEN** the IM service starts without registering that channel's adapter, and no inbound or outbound activity occurs for it

#### Scenario: feishu is one registered channel

- **WHEN** feishu is enabled
- **THEN** `feishu` is one active channel in the IM service's registry alongside `web` (whose frontend remains the WebUI), and removing feishu's adapter requires no change to core session routing

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
