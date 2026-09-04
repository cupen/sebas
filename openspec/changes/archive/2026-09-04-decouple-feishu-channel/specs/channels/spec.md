# channels Specification

## Purpose

Defines the core's neutral channel abstraction: a channel-agnostic session key, inbound event model, outbound presentation model, and adapter registry. The core depends only on this abstraction; concrete channels (Feishu, WebUI, future IM/agent clients) plug in as adapters without reshaping core domain types. Terminology (session, project, workbench, channel, adapter, execution body) is defined in `openspec/glossary.md`.

## ADDED Requirements

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

The core SHALL accept inbound events in a channel-neutral model: text, media, button callback, and form callback. Each event SHALL carry its originating `ChannelKey`. The router SHALL dispatch on the neutral event model only and SHALL NOT depend on any concrete channel's event type.

#### Scenario: text from any channel routes identically

- **WHEN** a text event arrives with channel key `feishu:oc_x`
- **THEN** the router treats it as the same text event as one with channel key `web:w1`, differing only in the key
- **AND** the channel-specific reply target (message id / thread root) is carried as channel-neutral metadata, not as a first-class field

#### Scenario: button callback maps to channel action

- **WHEN** a button callback arrives from channel `feishu`
- **THEN** the router dispatches the neutral button action and lets the feishu adapter resolve the concrete Feishu callback reference back to the session

### Requirement: Neutral outbound presentation

The core SHALL emit assistant output as a neutral outbound presentation model — the channel-agnostic equivalent of the current card model: a per-turn presentation instance that streams updates, freezes at turn end, and carries interactive elements (buttons, forms, selects) whose actions resolve to neutral events. Concrete channels SHALL render this model into their native presentation (e.g. Feishu card schema 2.0 JSON).

#### Scenario: streaming presentation coalesces

- **WHEN** multiple content deltas for one turn arrive within a debounce window on any channel
- **THEN** the core flushes them as a single coalesced presentation update for that channel

#### Scenario: interactive element resolves to neutral action

- **WHEN** a user activates an interactive element on a rendered presentation (e.g. clicks a permission button)
- **THEN** the channel adapter translates it into a neutral button-callback event addressed to the originating session

### Requirement: Adapter registry

The core SHALL maintain an adapter registry mapping channel names to their adapters. A channel's adapter SHALL be active only when its configuration enables it; the registry SHALL be queryable for which channels are active and their health. Registering a new channel SHALL NOT require changing core routing or domain types.

#### Scenario: disabled channel is absent from registry

- **WHEN** a channel's configuration disables it and provides no credentials
- **THEN** the core starts without registering that channel's adapter, and no inbound or outbound activity occurs for it

#### Scenario: feishu is one registered channel

- **WHEN** feishu is enabled
- **THEN** `feishu` is one active channel in the registry alongside `web`, and removing feishu's adapter requires no change to core session routing

## REMOVED Requirements

### Requirement: Feishu types are core domain types

**Reason**: This behavior is superseded by the neutral channel abstraction. `SessionKey` (Feishu `chat_id`+`thread_id`), `FeishuIn` event variants, and the Feishu card model previously acted as the core's session key, inbound event model, and outbound presentation. They are being replaced by `ChannelKey`, neutral events, and the neutral presentation model, with Feishu shapes confined to the feishu adapter.

**Migration**: Core domain types (`SessionKey`, `FeishuIn`, card model) are removed from core-facing interfaces. The feishu adapter implements the neutral abstraction and translates Feishu wire shapes at its boundary. `core_channel` protocol types are updated to the neutral shapes in the same change (see `core-session-channel`).