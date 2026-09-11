## Purpose

Defines the execution node: a standalone `sebas-node` executable that runs agent sessions on a machine other than the control plane, dials out to the control plane over a websocket, and owns the execution facts of its own sessions — child-process lifetime, the ordered turn log, provider credentials, material placement — while taking session identity and desired state from the control plane.

## ADDED Requirements

### Requirement: Execution node process persona

The system SHALL provide the execution node as a **standalone executable of its own** — not as a subcommand of the control-plane binary — which runs agent sessions on a host other than the control plane. Installing and running a node SHALL NOT require or carry the control-plane process roles (core, webui, router, im): the node artifact contains only what running the node's own sessions needs. The node SHALL dial out to the configured control plane over a websocket and SHALL NOT require the control plane to reach it inbound. The node SHALL keep its sessions and their child processes alive across link loss and across control-plane restarts.

#### Scenario: node installs without the control plane

- **WHEN** an operator installs and runs a node on a machine
- **THEN** the node executable is the only program required, and no control-plane role (core / webui / router / im) is present or runnable on that machine

#### Scenario: node dials out and comes online

- **WHEN** `sebas-node` starts with a paired credential and the configured control plane is reachable
- **THEN** it establishes an outbound websocket, completes the handshake, and is reported online
- **AND** no inbound port on the node is required

#### Scenario: sessions survive the control plane going away

- **WHEN** the websocket drops or the control plane restarts while sessions are running on the node
- **THEN** the node keeps those sessions and their child processes alive and continues their in-flight turns
- **AND** reconnection re-attaches the control plane to the same sessions instead of creating new ones

#### Scenario: unpaired node refuses to start

- **WHEN** `sebas-node` starts with neither a join token nor a stored credential
- **THEN** it exits as a startup failure naming the missing pairing, and starts no session

### Requirement: Stable node identity

Node identity SHALL be chosen or accepted by the operator at pairing time and persisted on the node, so that re-pairing after a reinstall can reuse the same node id. Projects and sessions SHALL address the node by this id. A node SHALL serve exactly one control plane.

#### Scenario: reinstall keeps the identity

- **WHEN** an operator reinstalls a node and pairs it with its previous node id
- **THEN** project entries that name that node remain addressable
- **AND** the node's history continues under a new epoch rather than being appended to the previous timeline

#### Scenario: conflicting node id is rejected

- **WHEN** a pairing presents a node id that is already online
- **THEN** the pairing is rejected with a cause naming the conflict and no second node comes online

### Requirement: Pairing credential lifecycle

Pairing SHALL use a one-time, expiring join token issued by the control plane; the node SHALL exchange it for a long-lived node credential. The credential SHALL be revocable by the control plane, and a revoked node SHALL be refused on its next connection attempt with a cause naming the revocation.

#### Scenario: join token is single use

- **WHEN** a join token has been consumed by a successful pairing
- **THEN** a second pairing presenting the same token is rejected with a cause naming the consumed token

#### Scenario: revoked node is refused

- **WHEN** the control plane revokes a node credential and that node attempts to connect
- **THEN** the connection is refused with a cause naming the revocation
- **AND** the control plane reports that node's sessions as unreachable rather than live

### Requirement: Handshake carries protocol version and capability manifest

Every connection SHALL begin with a handshake carrying the node's protocol version and a capability manifest: the configured agent kinds with their probed reachability, the provider inventory, and — per execution body — whether that body can enforce a session mode. The control plane SHALL refuse an unsupported protocol version instead of operating partially.

#### Scenario: incompatible version is refused honestly

- **WHEN** a node's protocol version is not supported by the control plane
- **THEN** the connection is refused with a cause naming both versions
- **AND** no session is accepted over that connection

#### Scenario: manifest drives selection

- **WHEN** a node reports its agent kinds with reachability
- **THEN** the control plane offers only the reachable kinds for sessions placed on that node

#### Scenario: mode enforceability is declared, not assumed

- **WHEN** a node reports that an execution body cannot enforce session modes
- **THEN** that limitation is part of the manifest and surfaces to the operator rather than being discovered at runtime

### Requirement: Provider credentials are node-local by default

The node SHALL use its own provider configuration and credentials by default, and SHALL report its provider inventory to the control plane. A provider or model chosen on the control plane SHALL be a desired value that the node applies and reports back as effective. The node MAY instead be configured to reach the control plane's router as its upstream, in which case it holds no provider credentials at all.

#### Scenario: control-plane selection is desired, not assumed

- **WHEN** the control plane selects a provider or model that the node cannot apply
- **THEN** the node reports the effective value it actually uses and the difference is visible, rather than the selection being echoed as applied

#### Scenario: proxied upstream holds no credentials

- **WHEN** the node is configured with the control-plane router as its upstream
- **THEN** the node stores no provider credentials and its agents' model traffic is routed through that upstream
- **AND** the node reports that model calls depend on the control plane being reachable

### Requirement: Operator-level materials are pulled and version-pinned

Project-level materials (project instructions and in-repository skills and subagent definitions) SHALL stay with the project tree on the node and SHALL NOT be transported. Operator-level materials (global skills, cross-project memory, global subagent definitions) SHALL be pulled from the control plane by the node when a session is spawned, placed where the session's execution body reads them, and pinned to the version in force at session creation. A control-plane change notification SHALL carry only the version or invalidation signal, never the content.

#### Scenario: pinned version survives a later change

- **WHEN** the control plane updates an operator-level material after a session was created
- **THEN** that session continues with the version pinned at its creation, and new sessions use the new version

#### Scenario: placement is per execution body

- **WHEN** a node materializes operator-level materials for a session
- **THEN** it places them where that session's execution body reads them, and reports a cause instead of silently ignoring materials when that body has no material location

#### Scenario: reading the placement is not guaranteed

- **WHEN** materials have been placed on the node
- **THEN** the system SHALL state that it guarantees the placement is current, not that the agent has read it

### Requirement: Node-local session log is ordered and epoch-stamped

The node SHALL keep, per session, an append-only log ordered by a monotonically increasing sequence number, together with an epoch that changes whenever that log is reset or lost. The log SHALL be the source for any fidelity the control plane requests, independently of the granularity at which events are streamed.

#### Scenario: log survives control-plane absence

- **WHEN** the control plane is unreachable while a session's turns complete
- **THEN** the node's log holds those turns in order and serves them to the control plane after reconnection

#### Scenario: reset is reported as a new epoch

- **WHEN** a node's log is reset or lost and the node reconnects
- **THEN** it reports a new epoch for that session, and the control plane marks the timeline as discontinuous instead of appending to the old one

### Requirement: Log retention leaves a reclaim watermark

The node MAY reclaim its local log according to its own retention policy. Every reclaim SHALL report a watermark naming the sequence number reclaimed through, and the control plane SHALL mark the corresponding history as unavailable rather than leaving a gap that reads as pending. When the node's storage ceiling is reached, the node SHALL refuse new sessions with a cause instead of discarding history.

#### Scenario: reclaimed segment is marked unavailable

- **WHEN** a node reclaims log segments the control plane has not yet pulled and reports the watermark
- **THEN** the control plane marks that range as unavailable-at-node rather than pending

#### Scenario: storage ceiling refuses new work

- **WHEN** a session is requested on a node whose storage ceiling has been reached
- **THEN** the request is rejected with a cause naming the storage condition
- **AND** no existing session's history is discarded to make room

### Requirement: Concurrency ceiling with honest rejection

The node SHALL enforce a configured maximum number of concurrent sessions. A session requested beyond the ceiling SHALL be rejected with a cause naming the limit; the node SHALL NOT queue it silently and SHALL NOT displace existing sessions.

#### Scenario: over-ceiling request is rejected

- **WHEN** a session is requested on a node already running its maximum number of sessions
- **THEN** the request is rejected with a cause naming the concurrency limit
- **AND** existing sessions keep running

### Requirement: The node never grants a permission itself

The node SHALL NOT hold any local decision surface that can grant a tool permission, and SHALL NOT resolve a parked permission request on its own. Permission decisions SHALL arrive only from the control plane. This SHALL hold even when the control plane is unreachable.

#### Scenario: no local approval path

- **WHEN** a permission request is parked on the node and the control plane is unreachable
- **THEN** the request stays parked; the node neither allows nor denies it locally

#### Scenario: a compromised node cannot self-approve

- **WHEN** a node is compromised and attempts to resolve a parked permission request
- **THEN** no decision path exists for it to do so
