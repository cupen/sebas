## ADDED Requirements

### Requirement: State directory derives every state path

The system SHALL resolve every persisted state location — the layered state databases, the WebUI user store, the archive registry, the project registry, the node-link registry, and the watchdog's service-override file — from a single state directory. The state directory SHALL be settable by one environment variable and SHALL otherwise default to one well-known location, and every derived path SHALL be a fixed filename inside it. A per-file environment variable SHALL remain honored as an explicit override of that one file, taking precedence over the derived path. No state location SHALL default outside the state directory. The retired single-database variable SHALL NOT be honored, so that a leftover value cannot silently point state at a file the system no longer uses.

#### Scenario: one variable relocates every state file

- **WHEN** only the state-directory variable is set
- **THEN** every state file and database resolves inside that directory
- **AND** no state file is created or read outside it

#### Scenario: a per-file override wins over the derived path

- **WHEN** the state-directory variable and one per-file variable are both set
- **THEN** that one file resolves to the per-file value
- **AND** every other file still resolves inside the state directory

#### Scenario: defaults all live under the default state directory

- **WHEN** neither the state-directory variable nor any per-file variable is set
- **THEN** every state path resolves inside the default state directory
- **AND** no state path default points somewhere else, so there is exactly one rule rather than a table of exceptions

#### Scenario: no state write escapes the derived directory

- **WHEN** the state directory is set and the process runs its normal lifecycle including first start, mutations, and shutdown
- **THEN** every state file that is created or modified lies inside it
- **AND** a file outside it is a defect, reported by the mechanical check

#### Scenario: a file with no previous override becomes pinnable

- **WHEN** the state directory is set and the service-override file or the node-link registry is written
- **THEN** it is written inside the state directory
- **AND** it can be relocated without editing any configuration file

#### Scenario: the retired database variable has no effect

- **WHEN** the retired single-database variable is exported while the state directory is configured
- **THEN** no database is opened at the retired variable's path
- **AND** the layered databases resolve from the state directory as normal
