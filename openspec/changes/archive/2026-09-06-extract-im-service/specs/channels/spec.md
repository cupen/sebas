# channels Specification（delta）

## MODIFIED Requirements

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
