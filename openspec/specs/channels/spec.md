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

The system SHALL keep a channel-neutral outbound presentation model — the channel-agnostic card model: a per-turn presentation instance that streams updates, freezes at turn end, and carries interactive elements (buttons, forms, selects) whose actions resolve to neutral events. The IM service SHALL build and maintain this presentation from the core's session content (session events and turn entries); the core SHALL NOT hold any IM-facing presentation state that reaches a channel. (Reality note: the core engine still computes an internal card state whose chat-facing `Out` instructions are dropped by the feishu-less outbound pump — a dead path scheduled for removal, not a presentation surface.) Concrete channels SHALL render this model into their native presentation (e.g. Feishu card schema 2.0 JSON).

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
