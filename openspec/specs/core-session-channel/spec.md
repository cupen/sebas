## Purpose

Define the channel between the sebas core — the sole owner of session state and
the sole spawner of ACP children — and out-of-process clients that need to
observe and drive sessions, so that a detached WebUI can be genuinely live and
genuinely drivable without becoming a second writer of session state.

Session identity on the wire is channel-neutral: the protocol carries a `ChannelKey` (channel name plus an opaque per-channel reference) rather than any concrete channel id shape (decouple-feishu-channel).
## Requirements

### Requirement: Core is the single session authority

The core process SHALL remain the only owner of session mapping state and the
only spawner of ACP child processes. A channel client SHALL NOT construct its
own `RouterHandle`, mutate mapping state locally, or spawn children; every
mutation a client wants SHALL be requested over the channel and applied by the
core. Client-side data SHALL be treated as a cache of core state with no
independent authority.

#### Scenario: client mutation is applied by the core

- **WHEN** a channel client requests that a session be created
- **THEN** the core creates it, applies the mapping change in its own state, and
  the client learns the result from the core's response and event stream rather
  than from a local mutation

#### Scenario: client holds no independent state

- **WHEN** a client and the core disagree about a session's status
- **THEN** the core's value is authoritative and the client replaces its cached
  value on the next snapshot or event

### Requirement: Channel transport and authentication

The core SHALL expose the channel on a Unix domain socket created with owner-only
permissions (0600) at a configurable path defaulting to `~/.sebas/core.sock`.
Every connection SHALL be authenticated by both peer credentials — the connecting
uid MUST equal the core's own uid — and a shared secret supplied out of band, in
the same posture as the watchdog control RPC. A connection failing either check
SHALL be rejected and closed without processing any request. The channel SHALL
NOT be exposed over TCP.

#### Scenario: foreign uid rejected

- **WHEN** a process running as a different uid connects to the socket
- **THEN** the connection is rejected and closed, and no request on it is
  processed

#### Scenario: missing or wrong secret rejected

- **WHEN** a connection presents no secret or a secret that does not match
- **THEN** the connection is rejected and closed, and no request on it is
  processed

#### Scenario: stale socket file is reclaimed

- **WHEN** the core starts and a socket file already exists at the path with no
  live listener behind it
- **THEN** the core removes the stale file and binds a fresh socket

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

### Requirement: Turn content retrieval

The channel SHALL provide a method returning the rendered turn content the core
holds for a given session, so a client can display an agent conversation it did
not itself receive. The response SHALL carry a monotonic position so a client can
request only what it has not yet seen.

#### Scenario: incremental turn fetch

- **WHEN** a client requests turn content for a session with a position it has
  already seen
- **THEN** the response contains only content after that position, with a new
  position to use next time

### Requirement: Honest degradation when the core is unreachable

When the channel cannot be reached — socket absent, connection refused, secret
rejected, or the connection dropped — a client SHALL surface that condition with
its cause and SHALL NOT present stale data as current, report a mutation as
succeeded, or offer a control whose request cannot be delivered. A client SHALL
reconnect on its own and resume with a fresh snapshot when the core returns.

#### Scenario: core down is stated, not hidden

- **WHEN** the core is not running and a client renders a session view
- **THEN** the view states that the core is not connected and why, rather than
  rendering an empty or stale list as though it were current

#### Scenario: no unsendable controls

- **WHEN** the channel is unreachable
- **THEN** controls that would require a channel request are unavailable and
  labeled with the reason, and no such request reports success

#### Scenario: reconnect resumes from a snapshot

- **WHEN** the core restarts while a client is connected
- **THEN** the client reconnects, takes a fresh snapshot, and its view converges
  on the core's state without a manual reload

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
