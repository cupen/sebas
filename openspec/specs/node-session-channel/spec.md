# node-session-channel Specification

## Purpose
Defines the session protocol between the control plane and an execution node: how a session is addressed and driven over the websocket link, at what granularity turn output travels, and how the two sides reconcile after a dropped link or a control-plane restart — state as an idempotent snapshot, history as an ordered append.

## Requirements

### Requirement: The protocol is session-granular

The link SHALL carry session-level operations: the control plane sends prompts, cancellation, close, and mode/model desired values; the node owns how a session and its turns are composed on its side, and returns turn output and session state. The protocol SHALL NOT carry per-driver internal vocabulary across the link.

#### Scenario: a prompt drives a remote session

- **WHEN** the control plane sends a prompt for a session placed on a node
- **THEN** the node composes the turn on its side and returns turn output and the resulting session state
- **AND** the control plane does not need to know which driver the node used

#### Scenario: capabilities beyond prompt and stream need protocol support

- **WHEN** a session-level capability (model switch, cancellation, usage, attachments) is offered to remote sessions
- **THEN** it is carried by an explicit protocol operation, and a capability the protocol does not carry is reported unavailable rather than silently missing

### Requirement: Session identity is issued by the control plane

Each session SHALL be identified by an id issued by the control plane, namespaced by project. The node SHALL key its local session and log by that id. On reconnection the control plane SHALL re-attach to sessions by that id and SHALL NOT create a replacement session for one that is still alive on the node.

#### Scenario: re-attach instead of re-create

- **WHEN** the control plane reconnects and a session with a known id is still running on the node
- **THEN** the control plane re-attaches to it and its history continues under the same identity

#### Scenario: unknown id is not a duplicate

- **WHEN** the control plane asks about a session id the node does not hold
- **THEN** the node reports it as unknown and the control plane does not silently create a new session under that id

### Requirement: Turn output is coalesced in transport, exact in the log

Turn output SHALL be emitted to the control plane coalesced over a short time window, and the node SHALL retain the exact ordered sequence in its log. When the coalescing buffer overflows, the node SHALL report that a segment was coalesced and can be retrieved, rather than dropping it silently. The control plane SHALL be able to retrieve any segment at full fidelity by sequence range.

#### Scenario: live output arrives coalesced

- **WHEN** a remote turn streams output
- **THEN** the control plane receives it in coalesced batches at roughly the configured window, and the session reads as streaming rather than as a black box

#### Scenario: overflow is reported, not hidden

- **WHEN** the node's coalescing buffer overflows during a turn
- **THEN** the control plane is told that a segment was coalesced and can be retrieved
- **AND** the node's log still holds the exact sequence

#### Scenario: full fidelity on demand

- **WHEN** the control plane requests a sequence range of a session's log
- **THEN** the node returns that range exactly as recorded

### Requirement: Reconcile uses two primitives only

Reconciliation SHALL consist of an idempotent state snapshot plus an incremental ordered-log pull from the control plane's cursor. Reconnecting a dropped link and reconnecting after a control-plane restart SHALL be the same operation, and neither SHALL duplicate, reorder, or split a session's history.

#### Scenario: reconnect converges

- **WHEN** the link is re-established after an absence
- **THEN** the node reports current session state and serves the log from the control plane's cursor onward
- **AND** applying the result twice yields the same control-plane view as applying it once

#### Scenario: repeated reconcile is harmless

- **WHEN** reconciliation runs while nothing has changed
- **THEN** the control plane's view is unchanged and no duplicate history appears

### Requirement: Parked approval requests are part of reconciliation

The node SHALL include its outstanding permission requests in reconciliation, so that a control plane returning after an absence can present and resolve every request still parked on the node. A request that was answered while the link was down SHALL NOT be presented again.

#### Scenario: parked requests reappear after a restart

- **WHEN** the control plane restarts while permission requests are parked on a node
- **THEN** reconciliation reports those requests and the operator can resolve them

#### Scenario: resolved requests are not resurrected

- **WHEN** a parked request was resolved before the control plane reconnected
- **THEN** reconciliation does not report it as outstanding

### Requirement: Project references cross the link as node and path

A session's project SHALL cross the link as a reference to a directory path on the node. The judgement of whether that path is usable SHALL be made by the node that would spawn the session, and an unusable path SHALL be rejected with a cause naming the path and the condition.

#### Scenario: path is judged where the spawn happens

- **WHEN** the control plane asks a node to start a session in a project directory
- **THEN** the node validates the path on its own filesystem before spawning
- **AND** a missing or unusable path is rejected with a cause rather than spawning elsewhere

#### Scenario: the same path on two hosts is not the same project

- **WHEN** two nodes both hold a directory at the same path
- **THEN** those are two distinct project references and neither is treated as the other

### Requirement: Session placement is a function of the project

A session SHALL be placed on the node named by its project reference. A session with no project SHALL be placed on the configured default execution node. When the target node is offline, the session request SHALL fail with a cause naming the node and its state; the request SHALL NOT be queued for later delivery.

#### Scenario: project decides the node

- **WHEN** the operator starts a session from a project registered against a node
- **THEN** the session is placed on that node

#### Scenario: project-less session uses the default node

- **WHEN** a session originates without a project directory
- **THEN** it is placed on the configured default execution node and its origin remains visible

#### Scenario: offline node fails honestly

- **WHEN** a session is requested for a project whose node is offline
- **THEN** the request fails with a cause naming the node and its state
- **AND** no placeholder session is created that implies the work has begun
