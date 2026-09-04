# core-session-channel Specification

## Purpose

Define the channel between the sebas core — the sole owner of session state and
the sole spawner of ACP children — and out-of-process clients that need to
observe and drive sessions, so that a detached WebUI can be genuinely live and
genuinely drivable without becoming a second writer of session state.

In this change the channel protocol's session identity is generalized from the
Feishu-shaped `SessionKey` to the channel-neutral `ChannelKey` (see `channels`),
so the same protocol carries sessions from any channel (feishu, web, future
IM/agent clients).

## MODIFIED Requirements

### Requirement: Channel protocol uses the neutral session key

The channel SHALL identify sessions by the channel-neutral `ChannelKey` carried
as a structured field — the channel name plus a channel-specific opaque
reference — encoded for transport. The protocol SHALL NOT expose concrete
channel id shapes (e.g. Feishu `chat_id` / `thread_id`) as first-class fields.
A client SHALL address a session by its `ChannelKey` and SHALL treat the
channel-specific reference as opaque. This replaces the previously Feishu-shaped
`SessionKey` (chat id + thread id) on the wire.

#### Scenario: sessions are addressed by neutral key

- **WHEN** a session originates from channel `feishu` with reference `oc_x` and
  thread `t1`
- **THEN** the channel protocol carries it as `ChannelKey`, and a client can
  display, message, close, or subscribe to it via that key without knowing the
  Feishu id shapes
- **AND** a session originating from channel `web` with reference `w1` is
  carried by the same protocol with a different channel name

#### Scenario: opaque reference is preserved

- **WHEN** a client sends a message addressed to a `ChannelKey`
- **THEN** the core routes it to the session whose key matches, and the
  channel-specific reference is passed through uninterpreted by the core

### Requirement: Session observation methods

The channel SHALL provide a snapshot method returning every known session with
the fields the WebUI renders — channel key (including the channel name and
per-channel reference), session id, status, phase, last-active — and a
subscription method that streams session events for the life of the connection.
A subscriber SHALL receive a snapshot first, then events, so that no event is
missed between the two.

#### Scenario: snapshot precedes the stream

- **WHEN** a client subscribes
- **THEN** it receives the current session set before any subsequent event, and
  applying the events in order to that set reproduces the core's current state

#### Scenario: session change reaches subscribers

- **WHEN** a session is created, changes status or phase, or is removed in the
  core
- **THEN** every live subscriber receives a corresponding event

### Requirement: Session drive methods

The channel SHALL provide methods to create a session with a prompt and an
optional project directory, send a message to an existing session, and close a
session. Each SHALL return either the resulting `ChannelKey` or a typed
rejection naming the reason. A create request carrying a project directory SHALL
have that path canonicalized and verified to be an existing directory before any
child is spawned, and SHALL be rejected otherwise. Spawned sessions SHALL be
registered under the requesting client's channel (e.g. `web` for the WebUI).

#### Scenario: create spawns a real session

- **WHEN** a client requests session creation with a prompt
- **THEN** the core spawns a real ACP session under the client's channel, returns
  its `ChannelKey`, and subscribers observe the new session

#### Scenario: unusable project directory rejected

- **WHEN** a create request names a path that is not an existing directory
- **THEN** the request is rejected with a reason and no child is spawned

#### Scenario: message to unknown session rejected

- **WHEN** a message or close request names a `ChannelKey` the core does not know
- **THEN** the response is a typed rejection and nothing is mutated