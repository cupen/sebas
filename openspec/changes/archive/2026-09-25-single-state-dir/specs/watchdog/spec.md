## ADDED Requirements

### Requirement: Service override layer follows the state directory

The watchdog's service desired-state override layer SHALL resolve inside the state directory rather than a fixed home-relative path, and SHALL be explicitly overridable for deployments that keep it elsewhere. Because this file was previously unpinnable, a sandboxed or test instance SHALL be able to keep it out of the operator's real configuration directory without editing any configuration file. The override layer is operator configuration, on par with the configuration file: it records explicit decisions made through service-set operations, and the watchdog's entire handling of it is reading it at spawn decisions and rewriting it when such an operation arrives. The watchdog SHALL NOT acquire a persistence layer for it — no database, no schema machinery; it stays a plain file. Service-set operations arrive at the watchdog's own control socket from surfaces such as the CLI, the web UI's service page, and the IM bridge; the core is not on that path. The three-layer resolution (configuration, then override file, then runtime) and the rule that core is unconditionally managed and ignores the override layer are unchanged.

#### Scenario: sandboxed watchdog does not touch the operator's directory

- **WHEN** the state directory is pinned and the watchdog starts with its override layer defaulting to the derived location
- **THEN** the override layer is created inside the pinned directory
- **AND** nothing is written to the operator's real configuration directory

#### Scenario: explicit override still wins

- **WHEN** the override layer's own path is configured explicitly
- **THEN** that path is used instead of the derived one

#### Scenario: the watchdog acquires no persistence layer

- **WHEN** a service-set operation is recorded
- **THEN** it is written to the plain override file and nowhere else
- **AND** the watchdog binary carries no database dependency

#### Scenario: a runtime override survives a watchdog restart

- **WHEN** the operator sets a service's desired state and the watchdog later restarts
- **THEN** the spawn decision for that service still reflects the recorded override
- **AND** the override was read from the file, not from any other process

#### Scenario: the override layer works while core is down

- **WHEN** the core process is not running and the operator sets a service's desired state
- **THEN** the watchdog records the desired state and applies it without involving the core
- **AND** the recorded state is read at the next spawn decision even if the core has never started

#### Scenario: core still ignores the override layer

- **WHEN** the override layer names core as disabled
- **THEN** core is still spawned and supervised, and the override is ignored with a warning
- **AND** this is unchanged from the previous behavior
