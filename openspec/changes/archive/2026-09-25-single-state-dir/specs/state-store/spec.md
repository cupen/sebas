## MODIFIED Requirements

### Requirement: Database location and single-writer ownership

The domain state SHALL live in a set of purpose-layered SQLite databases inside a single state directory, each opened in WAL mode, rather than in one undifferentiated database. Layering SHALL follow a two-level rule: first by **writing process** — each database SHALL have exactly one writer, and a process that is not a database's writer SHALL NOT open it, accessing that state exclusively through the core channel state methods; then, within the core's own databases, by **growth characteristic** — bounded system configuration (providers, model aliases, card and runtime settings, whose row count is decided by hand-written configuration) SHALL be separated from user data that grows with use (projects, the session map, and the session and message content that follows it). Paths SHALL expand a leading `~/`. All mutations SHALL be applied by the owning process's state store, serialized one at a time per database. Because the databases are layered, an operation that rebuilds or resets one of them SHALL NOT affect the other.

#### Scenario: Environment override relocates the database

- **WHEN** the environment variable for one database points to a custom path
- **THEN** the owning process opens that database at that path
- **AND** the other databases keep resolving inside the state directory

#### Scenario: Tilde paths expand to home

- **WHEN** a configured database path begins with `~/`
- **THEN** the path is expanded to the user's home directory before use

#### Scenario: Concurrent mutations serialize

- **WHEN** two clients issue state mutations concurrently
- **THEN** both apply in serialization order and a later snapshot reflects the combined result — never a torn or lost update without an explicit error

#### Scenario: bounded configuration is separated from growing user data

- **WHEN** the databases are inspected after extended use
- **THEN** provider, model-alias, and settings rows live in the bounded configuration database
- **AND** projects and the session map live in the user-data database
- **AND** the configuration database's size is not driven by how much the user accumulates

#### Scenario: one database per writer

- **WHEN** the set of databases and their writers is inspected
- **THEN** each database is written by exactly one process
- **AND** no database is opened by a process other than its writer

#### Scenario: resetting user data leaves configuration intact

- **WHEN** the user-data database is rebuilt because its structure diverged from the models
- **THEN** the configuration database is untouched
- **AND** settings survive the rebuild

#### Scenario: a database's unavailability is reported, not hidden

- **WHEN** one database cannot be opened while another opens successfully
- **THEN** the unavailable domain reports an explicit unavailable state naming the cause
- **AND** the process does not present substitute or default values as if the state were current
