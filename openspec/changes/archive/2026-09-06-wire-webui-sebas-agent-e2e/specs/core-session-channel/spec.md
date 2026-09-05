## MODIFIED Requirements

### Requirement: Session drive methods

The channel SHALL provide methods to create a session with a prompt, an
optional project directory, and an optional execution-body hint (`native` for
the native agent kernel, `acp` for the ACP bridge), send a message to an
existing session, and close a session. Each SHALL return either the resulting
`ChannelKey` or a typed rejection naming the reason. A create request carrying
a project directory SHALL have that path canonicalized and verified to be an
existing directory before any child is spawned, and SHALL be rejected
otherwise. Spawned sessions SHALL be registered under the requesting client's
channel (e.g. `web` for the WebUI). The core SHALL route the session's
execution according to the hint — `native` sessions run inside the core's
native kernel, `acp` sessions spawn an ACP child — and a request without a
hint SHALL default to `acp`, preserving behavior for existing clients.

#### Scenario: create spawns a real session

- **WHEN** a client requests session creation with a prompt
- **THEN** the core spawns a real session under the client's channel, returns
  its `ChannelKey`, and subscribers observe the new session

#### Scenario: native hint routes to the native kernel

- **WHEN** a create request carries `backend = "native"` with a usable project
  directory
- **THEN** the session executes in the native kernel, appears in the snapshot
  with the native execution body, and no ACP child is spawned for it

#### Scenario: missing hint defaults to ACP

- **WHEN** a create request from an older client carries no execution-body
  hint
- **THEN** the session is spawned on the ACP bridge exactly as before the hint
  existed

#### Scenario: native execution body unavailable is a typed rejection

- **WHEN** a create request carries `backend = "native"` and the native kernel
  cannot serve sessions (for example, no provider credentials configured)
- **THEN** the response is a typed rejection naming that cause and no session
  is created

#### Scenario: unusable project directory rejected

- **WHEN** a create request names a path that is not an existing directory
- **THEN** the request is rejected with a reason and no child is spawned

#### Scenario: message to unknown session rejected

- **WHEN** a message or close request names a `ChannelKey` the core does not
  know
- **THEN** the response is a typed rejection and nothing is mutated

### Requirement: Session observation methods

The channel SHALL provide a snapshot method returning every known session with
the fields the WebUI renders — channel key (including the channel name and
per-channel reference), session id, status, phase, last-active — plus the
session's execution body and its current model when one is set, and a
subscription method that streams session events for the life of the
connection. A subscriber SHALL receive a snapshot first, then events, so that
no event is missed between the two.

#### Scenario: snapshot precedes the stream

- **WHEN** a client subscribes
- **THEN** it receives the current session set before any subsequent event, and
  applying the events in order to that set reproduces the core's current state

#### Scenario: execution body and model are visible

- **WHEN** a client snapshots a session set containing both an ACP session and
  a native-kernel session with a model selected
- **THEN** each entry states its execution body, and the native entry states
  its current model

#### Scenario: session change reaches subscribers

- **WHEN** a session is created, changes status or phase, or is removed in the
  core
- **THEN** every live subscriber receives a corresponding event

## ADDED Requirements

### Requirement: Gated-call approval over the channel

The channel SHALL carry approval requests raised by native-kernel sessions —
gated tool calls awaiting an operator decision — to connected channel clients,
and SHALL carry the operator's decision (allow-once / allow-session / deny,
with an optional reason) back to the kernel's approver. A session whose
approval request has no reachable client SHALL fail closed: the call is not
executed. The approval surface SHALL be available in every process
configuration served through the channel, so a detached WebUI presents the
same review card as the in-process one.

#### Scenario: detached review card answers a gated call

- **WHEN** a native session requests approval for a gated tool call while a
  detached WebUI is connected to the channel
- **THEN** the review card is presented with the decision options, and an
  allow-once decision lets only that call through

#### Scenario: deny round-trips to the kernel

- **WHEN** the operator denies a channel-delivered approval with a reason
- **THEN** the kernel's approver receives the denial, the tool call is not
  executed, and the session reflects the rejection

#### Scenario: no reachable client fails closed

- **WHEN** a native session requests approval and no channel client is
  connected to receive it
- **THEN** the request is denied and the tool call is not executed

### Requirement: Session model selection over the channel

The channel SHALL provide a method to set the model of a known session,
returning either success or a typed rejection. Setting the model on a session
SHALL affect its subsequent turns regardless of the session's execution body.
A set-model request naming an unknown session, or a model the session's
execution body cannot serve, SHALL be rejected with a typed reason and nothing
SHALL change.

#### Scenario: model change reaches a native session

- **WHEN** a client sets a model on a native-kernel session over the channel
- **THEN** the session's current model updates and subsequent turns use it

#### Scenario: unknown session rejected

- **WHEN** a set-model request names a `ChannelKey` the core does not know
- **THEN** the response is a typed rejection and no session state changes

#### Scenario: unservable model rejected

- **WHEN** a client sets a model the session's execution body cannot serve
- **THEN** the response is a typed rejection naming the reason, and the
  session keeps its previous model
