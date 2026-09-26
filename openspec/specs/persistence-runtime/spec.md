# persistence-runtime Specification

## Purpose
Establishes the shared persistence runtime for the workspace: the connection recipe (journal mode, busy timeout, foreign-key enforcement), the schema self-description and startup reconciliation machinery, and the serialized single-writer execution model — provided once, by a neutral crate, so that every component which talks to SQLite reuses it instead of hand-copying a recipe or growing a second versioning mechanism. It also makes schema models declarable in any crate rather than only inside the main binary crate. This capability is an architectural contract about where the runtime lives and that it is reused; the behavior of any individual database's schema and migration remains governed by that database's own capability.

## Requirements

### Requirement: Persistence runtime is provided once and reused

The connection recipe, the schema reconciliation machinery, and the serialized single-writer execution model SHALL be provided by exactly one shared crate. Every component that opens a SQLite database SHALL obtain its connection from that crate rather than composing pragmas locally, and SHALL obtain schema reconciliation from it rather than running its own table-diffing or version-stamping logic. A component MAY choose which shared transaction behavior to use, but SHALL NOT roll its own connection setup.

#### Scenario: A second database does not re-implement the recipe

- **WHEN** a component other than the main core process needs a SQLite connection with the standard configuration
- **THEN** it calls the shared crate for that connection
- **AND** no local copy of the pragma sequence exists in that component

#### Scenario: No component carries a private migration mechanism

- **WHEN** the workspace is searched for schema-version stamping or table-column diffs
- **THEN** exactly one implementation exists, in the shared crate
- **AND** no component reconciles its own schema by an independently written procedure

#### Scenario: Shared runtime imposes no schema policy on its callers

- **WHEN** a component uses the shared crate for its connection and execution model
- **THEN** the shared crate does not force a schema-versioning scheme, a reset policy, or a transaction isolation behavior on it
- **AND** the component's existing behavior for those aspects is unchanged

### Requirement: Schema models are declarable outside the main binary crate

The attribute that declares a table's columns from a model struct SHALL be usable from any crate in the workspace, not only from the main binary crate. The generated code SHALL reference the shared persistence crate, so a crate that declares a schema model only needs a dependency on that crate.

#### Scenario: A model outside the main crate compiles

- **WHEN** a crate other than the main binary crate derives the schema-columns attribute on a model struct
- **THEN** the crate compiles without referencing the main binary crate
- **AND** the model's declared column set is identical to what the same struct would declare inside the main crate

#### Scenario: Column metadata is unchanged

- **WHEN** the same model struct is compiled before and after the attribute's generated path is relocated
- **THEN** the declared column names, affinities, defaults, and nullability are identical

#### Scenario: Unsupported declarations still fail at compile time

- **WHEN** a model declares an unsupported type, an unsupported attribute, or a non-null column without a default
- **THEN** compilation fails with the same class of diagnostic as before the relocation

### Requirement: Serialized single-writer execution is the shared model

The shared crate SHALL provide the serialized execution model in which database commands are applied one at a time by a single owner of the connection, with callers receiving results asynchronously. A component that requires serialized writes SHALL use this model instead of an ad-hoc mutex or a per-call connection. The model SHALL remain domain-agnostic: it SHALL NOT know about any domain table or type.

#### Scenario: Commands serialize through one owner

- **WHEN** two callers dispatch commands concurrently
- **THEN** the commands are applied one at a time by the single connection owner
- **AND** each caller receives its own result, without observing another caller's partial state

#### Scenario: The execution model carries no domain knowledge

- **WHEN** the shared crate's public surface is inspected
- **THEN** it names no domain table, row type, or domain concept
- **AND** a component can use it for a database whose tables it never sees

#### Scenario: A caller keeping a mutex keeps its behavior

- **WHEN** a component already serializes its own access through a mutex and is migrated to the shared crate for its connection only
- **THEN** its locking and isolation behavior is unchanged
- **AND** its existing concurrency tests pass unmodified

### Requirement: One struct per table with object-style CRUD

Every SQLite table SHALL correspond to exactly one struct, and one persisted row SHALL correspond to one instance of that struct. The mapping SHALL be declared on the struct — table name, columns derived from its fields, primary key — and the standard CRUD operations (save or upsert, find by primary key, list all, delete by primary key) SHALL be generated for it, so that a row is saved, fetched, and removed through object-style calls on the instance or its type rather than through per-table hand-written SQL or free functions. Queries that are not standard CRUD (aggregations, conditions on non-key columns) MAY remain hand-written SQL, but they SHALL return instances of the same struct rather than loose values. A consumer SHALL NOT store a table's rows as an untyped JSON map whose keys are matched by convention.

#### Scenario: A row is an instance that can save itself

- **WHEN** a struct mapped to a table has an instance with changed field values
- **THEN** saving that instance persists the change as that table's row
- **AND** reading the table back yields an instance with the same field values

#### Scenario: Standard CRUD needs no hand-written SQL

- **WHEN** a new table is introduced as a struct
- **THEN** its save, find, list, and delete operations come from the declaration alone
- **AND** no per-table SQL is written for those operations

#### Scenario: Adding a column updates the mapping, not call sites

- **WHEN** a field is added to a mapped struct and the schema reconciler adds the column
- **THEN** the generated CRUD carries the new field without any call-site change
- **AND** existing persisted rows read back with the new field at its default

#### Scenario: Non-standard queries still return instances

- **WHEN** a hand-written query selects rows by a non-key condition
- **THEN** it returns instances of the table's struct
- **AND** it does not return untyped maps or loose column values

#### Scenario: Untyped carriers are not the storage contract

- **WHEN** a table's rows are written or read
- **THEN** they pass through the mapped struct
- **AND** no table stores its rows as a JSON map interpreted by key-name convention

#### Scenario: Multi-record atomicity is explicit

- **WHEN** an operation must atomically change rows of more than one table
- **THEN** it is expressed as one serialized unit on the owning store
- **AND** it either commits wholly or leaves both tables unchanged
