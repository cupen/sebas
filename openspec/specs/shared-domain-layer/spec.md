# shared-domain-layer Specification

## Purpose
Establishes the neutral shared domain layer: the single place where cross-role domain concepts — session identity and status, session key encoding, provider and model descriptors, session events, turn entries, approval decisions, and the neutral primitives they need — are defined, so that the control plane and the execution node share one definition and one implementation instead of each carrying its own copy. A concept whose one canonical form is also a persisted row (the project record is the first) is defined once in the crate that owns the table instead, and the layer carries no copy of it; that placement rule is part of this contract, so that "one definition" never decays into "one definition plus a shadow copy in the layer". This capability is an architectural contract, not user-facing behavior: it constrains the workspace's shape so that later changes can reuse the layer rather than duplicate it again.

## Requirements

### Requirement: Single canonical definition per shared domain concept

The system SHALL define each shared domain concept — session identity and status, session key encoding, provider and model descriptors, session events, turn entries, approval decisions — in exactly one place, and every consumer SHALL use that definition rather than declaring its own. A consumer MAY re-export the canonical definition to preserve its existing public surface, but SHALL NOT redeclare the type or re-list its fields.

The workspace SHALL place each concept by its persistence nature. A concept whose single canonical form serves as both the domain object and a persisted row SHALL be defined in the crate that owns that table (today the project record, in the ActiveRecord crate), together with the rule that derives its identity; the neutral layer SHALL NOT keep a parallel copy of such a concept — not the type, not a re-listed field set, and not a constant that duplicates a column default or a storage key. A concept with no persisted form SHALL remain in the neutral layer.

#### Scenario: session key encoding has exactly one implementation

- **WHEN** the workspace is searched for implementations of the `channel\0reference` session key encoding
- **THEN** exactly one implementation exists, in the neutral channels crate
- **AND** every consumer (core node-link projection, web UI routes, dispatch engine, IM frontend, native bridge) calls it instead of computing the encoding locally

#### Scenario: adding a consumer adds no copy

- **WHEN** a further module begins to consume session keys
- **THEN** it depends on the canonical implementation and no second codec is introduced
- **AND** the test pinning byte-identical encoding for the former six call sites passes unchanged

#### Scenario: tilde expansion has one implementation

- **WHEN** two crates in different dependency branches need to expand a leading `~/` in a configured path
- **THEN** both call the shared implementation
- **AND** no consumer carries its own copy justified by an inability to reach the other's crate

#### Scenario: presentation views no longer re-list canonical fields

- **WHEN** a presentation view carries fields that also exist on the canonical type for the same concept
- **THEN** those fields are produced by one explicit conversion from the canonical type, not by a hand-written field-by-field listing

#### Scenario: a table-backed concept lives with its table

- **WHEN** a shared domain concept has exactly one canonical form that serves as both the domain object and a persisted row, as the project record does
- **THEN** that definition lives in the crate owning the table, next to the identity rule that derives the row's stable id
- **AND** the neutral layer declares no type, field listing, or constant of the same concept
- **AND** concepts with no persisted form stay in the neutral layer
- **AND** a duplicated constant standing in for a persisted column's default is removed rather than kept in sync by a test

### Requirement: Neutral leaf dependency

The shared domain layer SHALL be a leaf crate depending only on neutral primitives. It SHALL NOT depend on any control-plane role implementation (the core, web UI, router, or IM layer) and SHALL NOT depend on the execution node. It SHALL NOT depend on the persistence runtime, and SHALL NOT absorb a concept whose canonical form is a persisted row — the layer stays usable by a role that cannot link the persistence runtime. It SHALL be usable by both the control plane and the execution node without either pulling in the other.

#### Scenario: the shared layer carries no role implementation

- **WHEN** the dependency graph of the shared domain crate is listed
- **THEN** it contains no role implementation and no execution node

#### Scenario: adopting the shared layer does not break node isolation

- **WHEN** the execution node depends on the shared domain layer
- **THEN** `cargo tree -p sebas-node` still contains no control-plane role implementation
- **AND** the node artifact still contains only what running the node's own sessions needs

#### Scenario: the shared layer is reachable from every role

- **WHEN** any crate (root binary, web UI, dispatch, router, IM, node) needs a shared domain concept
- **THEN** it obtains that concept from the shared layer through a normal path dependency
- **AND** no crate is forced to copy a definition because the original is unreachable

#### Scenario: the shared layer does not take on persistence

- **WHEN** a concept's canonical form is a persisted row
- **THEN** the shared layer neither defines nor re-declares it, and adds no persistence runtime dependency to carry one
- **AND** the shared layer's dependency graph still contains no SQLite runtime
- **AND** a role that cannot link the persistence runtime keeps obtaining its own persistence-free concepts from the shared layer

#### Scenario: a shared protocol crate can be built on the layer without a cycle

- **WHEN** a crate carrying wire protocol types depends on the shared domain layer
- **THEN** each role that must speak that protocol can depend on it
- **AND** no dependency cycle is formed, because the protocol crate does not depend on any role implementation

### Requirement: Wire and on-disk compatibility preserved

The refactor SHALL NOT change any cross-process wire shape or any persisted on-disk shape. JSON field names and enum tags, newline-delimited framing, and SQLite table and column names and affinities SHALL remain byte-identical to the pre-refactor system. Any such change SHALL be an explicitly declared breaking change in its own change, never a side effect of relocating a definition.

#### Scenario: golden wire fixtures are unchanged

- **WHEN** the recorded wire fixtures for the session channel, node link, and WebUI WebSocket and HTTP surfaces are compared with post-refactor serialization
- **THEN** they are byte-identical

#### Scenario: persisted state still opens and survives

- **WHEN** an existing state database and the existing JSON state files are opened by the post-refactor binary
- **THEN** no schema difference is reported
- **AND** no row is dropped, reset, or rewritten

#### Scenario: an unrepresentable storage change is refused

- **WHEN** a step of this refactor would require renaming or removing a persisted column, or merging two persisted shapes into one
- **THEN** that step is rejected as out of scope and deferred, rather than performed silently

### Requirement: Shape boundaries use explicit typed conversions

Where one concept is legitimately carried in more than one shape (a canonical record and a presentation view, for example), the conversion between shapes SHALL be an explicit typed conversion, and both serialized shapes SHALL be pinned by a test. Untyped JSON values SHALL NOT be the contract between two shapes. A concept that is carried in the same form on the wire and on disk SHALL have exactly one definition rather than two parallel ones.

#### Scenario: shape boundaries are typed and pinned

- **WHEN** a concept is carried in more than one shape, such as a canonical record and a presentation view
- **THEN** a named conversion exists in the direction needed
- **AND** a test pins both serialized shapes, so that a field added on one side alone fails the test

#### Scenario: a concept with one form is defined once

- **WHEN** a concept is carried in the same form on the wire and on disk, as the project record is
- **THEN** it has exactly one definition shared by both forms
- **AND** no consumer maintains a parallel field listing of it

#### Scenario: drift is caught before release

- **WHEN** a field is added to either side of a typed conversion without updating the other
- **THEN** the conversion fails to compile or its pinning test fails

#### Scenario: untyped carriers stay internal

- **WHEN** a boundary is converted between two shapes
- **THEN** it is converted through the typed conversion, not by passing a bare JSON value whose keys are matched by convention
