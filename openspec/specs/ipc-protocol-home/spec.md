# ipc-protocol-home Specification

## Purpose
Establishes a single home for the workspace's cross-process protocol definitions, so that every role which speaks a protocol shares one definition instead of redeclaring it, and so that protocol evolution has explicit, mechanically enforced rules. It covers which crate the wire types live in, the dependency direction that keeps that crate reachable from every role, the compatibility status of the encoding and framing, and the forward-compatibility rules that a contract fixture set enforces.

## Requirements

### Requirement: Protocol definitions live in one reachable home

The wire types of every inter-process boundary SHALL be defined in one neutral crate that any role can depend on. A role that speaks a boundary's protocol SHALL use that crate's definitions and SHALL NOT redeclare the boundary's messages, frames, or handshake — including partial subsets of them. A role SHALL NOT construct protocol messages from ad-hoc JSON literals whose keys are matched by convention instead of by a shared type.

#### Scenario: No role carries a private copy of a protocol

- **WHEN** the workspace is searched for declarations of a boundary's message, frame, or handshake types
- **THEN** exactly one declaration exists, in the shared protocol crate
- **AND** no role declares a subset or a parallel variant of it

#### Scenario: The router speaks the channel through shared types

- **WHEN** the router subscribes to the core's state stream
- **THEN** it uses the shared protocol crate's frame type, handshake, and request type
- **AND** no locally declared frame subset, locally hand-rolled handshake, or inline JSON request literal remains in the router

#### Scenario: A protocol change is a compile error, not a silent divergence

- **WHEN** a field is added to or renamed in a shared protocol type
- **THEN** every role that produces or consumes it either fails to compile or fails a contract fixture
- **AND** no role can silently continue with a stale local copy

### Requirement: The protocol crate depends only on neutral leaves

The shared protocol crate SHALL depend only on neutral leaf crates and generic libraries. It SHALL NOT depend on any control-plane role implementation or on the execution node. This keeps the crate reachable from every role without forming a dependency cycle.

#### Scenario: Every role can depend on the protocol crate

- **WHEN** the root binary, the web UI, the dispatch layer, the router, the IM layer, or the execution node needs a protocol type
- **THEN** it obtains it from the shared protocol crate through a normal path dependency
- **AND** no dependency cycle is formed

#### Scenario: The protocol crate drags in no role implementation

- **WHEN** the dependency graph of the shared protocol crate is listed
- **THEN** it contains no role implementation and no execution node
- **AND** a mechanical check fails if one is added

### Requirement: Encoding and framing are a compatibility surface

The encoding and framing of each boundary SHALL be treated as a compatibility surface rather than an implementation detail. Changing a boundary's encoding or framing SHALL be an explicitly declared breaking change with a stated migration path for the peers it affects; it SHALL NOT be introduced as a side effect of another change.

#### Scenario: An encoding change cannot slip in unnoticed

- **WHEN** a change alters how a boundary encodes or delimits its messages
- **THEN** the change declares it as breaking and names how existing peers are handled
- **AND** the boundary's contract fixtures are updated in the same change, with the reason recorded

#### Scenario: Existing framing stays as it is

- **WHEN** a boundary already uses newline-delimited JSON, or one JSON object per websocket text frame
- **THEN** that framing is preserved unless a change explicitly declares otherwise

### Requirement: Forward compatibility rules are explicit and enforced

Protocol evolution SHALL follow stated rules, and those rules SHALL be enforced by checked-in contract fixtures rather than by convention:

- a newly added field SHALL carry a default so that peers which do not send it remain readable;
- an enum taking values from the wire SHALL retain an unknown-value path, so a value introduced by a newer peer does not fail decoding;
- removing a field or renaming a field or enum value SHALL be a declared breaking change.

The fixture set SHALL pin each boundary's serialized shape — both the serialized bytes of a representative payload and the set of field names — so that a removal or rename fails a test rather than reaching a release.

#### Scenario: Adding a field does not break older peers

- **WHEN** a field is added to a protocol message and a peer sends a payload without it
- **THEN** the payload still decodes and the field takes its default
- **AND** the boundary keeps working

#### Scenario: An unknown enum value does not fail the message

- **WHEN** a peer sends an enum value introduced by a newer version
- **THEN** the message is accepted and the value is surfaced as unknown
- **AND** the connection is not closed and no frame is dropped

#### Scenario: Removing or renaming a field fails the fixtures

- **WHEN** a field is removed from, or renamed in, a protocol type
- **THEN** the boundary's contract fixture test fails
- **AND** the change cannot land without declaring the break and updating the fixture

#### Scenario: Fixtures cover every protocol boundary

- **WHEN** the contract fixture set is inspected
- **THEN** the session channel, the node link, and the web UI's websocket and HTTP surfaces each have pinned shapes
- **AND** each pinned shape includes the set of field names, not only a serialized sample
