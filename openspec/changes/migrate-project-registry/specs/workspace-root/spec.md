## ADDED Requirements

### Requirement: Project registry is persisted in the core state store

The project registry — local projects and projects placed on remote execution nodes alike — SHALL be persisted in the core state store, which SHALL be its only authority. The registry SHALL carry the node dimension for every entry, so that a project's placement is part of its persisted identity rather than a property that exists only on disk somewhere else. Only the core process SHALL write the registry; every other role SHALL read and mutate it through the state methods. No separate project-registry file SHALL be written or read.

#### Scenario: a remote project survives a restart

- **WHEN** a project is added for a remote execution node and the process restarts
- **THEN** the project is present in the registry with the same node placement
- **AND** it was read from the state store, not from a file

#### Scenario: local and remote projects share one store

- **WHEN** both a local project and a remote-node project are registered
- **THEN** both are read from the state store in one listing
- **AND** no project entry is read from a separate file

#### Scenario: no project-registry file is written

- **WHEN** projects are added, renamed, reordered, or removed
- **THEN** no project-registry file is created or modified on disk

#### Scenario: placement is honoured from the persisted registry

- **WHEN** the registry is read after a restart
- **THEN** each entry's node placement is taken from the store
- **AND** a local project is distinguishable from one placed on a node

#### Scenario: a local project keeps its existing serialized spelling

- **WHEN** a local project is listed through any surface
- **THEN** its node placement serializes exactly as it did before the placement became a persisted column
- **AND** a client that does not care about placement sees an unchanged shape

### Requirement: Project surfaces degrade honestly when the store is unreachable

A surface that renders projects SHALL present an explicit unavailable state naming the cause when the state store cannot be reached. It SHALL NOT fall back to a file-derived project list, and SHALL NOT present a file-derived or previously cached list as the current registry. Mutating entries SHALL be disabled while the store is unavailable.

#### Scenario: unreachable store shows unavailable, not a file

- **WHEN** the core is not running and a project surface is opened
- **THEN** it presents an explicit unavailable state with the cause
- **AND** it does not show a project list read from a file

#### Scenario: mutations are disabled while unavailable

- **WHEN** the store is unreachable
- **THEN** project add, rename, reorder, and remove entry points are disabled
- **AND** no local file is written as a substitute

#### Scenario: recovery restores the registry

- **WHEN** the store becomes reachable again
- **THEN** the surface shows the registry read from the store
- **AND** the unavailable state is cleared

### Requirement: A project record has one canonical shape

A project record SHALL have exactly one canonical definition, used for both its persisted form and its wire form. A consumer SHALL NOT re-list the record's fields in a parallel declaration, and the two forms SHALL NOT diverge: the set of fields and their serialized names SHALL be stated in one place and pinned by a test, so that adding a field cannot update only one side.

#### Scenario: storage and wire shapes cannot drift

- **WHEN** a field is added to, removed from, or renamed in the project record
- **THEN** the change is made in the single definition
- **AND** the pinning test reflects both the persisted and the wire form, so a one-sided change fails

#### Scenario: no parallel field listing remains

- **WHEN** the workspace is searched for declarations of a project record
- **THEN** exactly one definition exists for the shared fields
- **AND** no consumer carries a hand-written field-by-field copy of it

#### Scenario: presentation-only fields stay out of the record

- **WHEN** a surface needs a display-only field (such as a computed label)
- **THEN** that field is produced by a conversion from the canonical record rather than added to it
- **AND** the record does not grow to carry presentation concerns
