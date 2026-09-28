## RENAMED Requirements

- FROM: `### Requirement: State directory derives every state path`
- TO: `### Requirement: Sebas home derives every file location`

## MODIFIED Requirements

### Requirement: Config discovery

Every subcommand SHALL take an explicit `--config`/`-c` path defaulting to
`<SEBAS_HOME>/config.toml` (the configuration file inside the sebas home);
there is no multi-path search and no `SEBAS_CONFIG` environment override. An
explicit `--config`/`-c` flag SHALL always win over the default, and a path
that is unreadable or absent keeps the existing env-only bootstrap behavior.
The router's reload source (`SEBAS_ROUTER_CONFIG`) SHALL keep precedence over
that same default when it is unset.

#### Scenario: default path

- **WHEN** `sebas core` is invoked with no `-c` while `SEBAS_HOME` is unset
- **THEN** the configuration file resolves to `<home>/.sebas/config.toml`
  and is loaded when present

#### Scenario: explicit flag wins

- **WHEN** `-c ./config.toml` is passed while `SEBAS_HOME` points elsewhere
- **THEN** the working-directory file is loaded
- **AND** no configuration is read from the sebas home

### Requirement: Sebas home derives every file location

The system SHALL resolve every sebas-owned file location — the layered state
databases, the WebUI user store, the archive registry, the project registry,
the node-link registry, the watchdog's service-override file, the
configuration file default, the channel secret file default, the channel
socket, the control socket, the media download cache, the sebas-node state
directory, and the watchdog's upgrade data directory — from a single sebas
home directory. `SEBAS_HOME` SHALL be the one environment variable that sets
it, and it SHALL otherwise default to `~/.sebas`. Derived paths SHALL be
fixed names inside it: databases and registry files flat in the home root,
sockets under `run/`, the media cache under `cache/downloads/`, node state
under `node/`, and upgrade data under `upgrade/`. The legacy alias
`SEBAS_STATE_DIR` SHALL still be honored when `SEBAS_HOME` is unset, with a
startup warning announcing the rename; when both are set `SEBAS_HOME` SHALL
win. A per-file environment variable or configuration key SHALL remain
honored as an explicit override of that one location, taking precedence over
the derived path; the channel secret file keeps its rule of defaulting
alongside the configuration file, which itself defaults inside the home. No
sebas-owned location SHALL default outside the sebas home. The retired
single-database variable SHALL NOT be honored, so that a leftover value
cannot silently point state at a file the system no longer uses.

#### Scenario: one variable relocates every state file

- **WHEN** only `SEBAS_HOME` is set
- **THEN** every sebas-owned file and socket — databases, registries,
  configuration default, sockets, caches, node and upgrade state — resolves
  inside that directory
- **AND** no sebas-owned file is created or read outside it

#### Scenario: a per-file override wins over the derived path

- **WHEN** `SEBAS_HOME` and one per-file variable are both set
- **THEN** that one file resolves to the per-file value
- **AND** every other file still resolves inside the sebas home

#### Scenario: defaults all live under the default state directory

- **WHEN** neither `SEBAS_HOME` nor any per-file variable is set
- **THEN** every sebas-owned path resolves inside the default sebas home
  `~/.sebas`
- **AND** no default points somewhere else, so there is exactly one rule
  rather than a table of exceptions

#### Scenario: no state write escapes the derived directory

- **WHEN** the sebas home is set and the process runs its normal lifecycle
  including first start, mutations, and shutdown
- **THEN** every sebas-owned file that is created or modified lies inside it
- **AND** a file outside it is a defect, reported by the mechanical check

#### Scenario: a file with no previous override becomes pinnable

- **WHEN** the sebas home is set and the service-override file, the
  node-link registry, a socket, the media cache, the node state directory,
  or the upgrade data directory is written
- **THEN** it is written inside the sebas home
- **AND** it can be relocated without editing any configuration file

#### Scenario: the retired database variable has no effect

- **WHEN** the retired single-database variable is exported while the sebas
  home is configured
- **THEN** no database is opened at the retired variable's path
- **AND** the layered databases resolve from the sebas home as normal

#### Scenario: the legacy alias still works but warns

- **WHEN** only `SEBAS_STATE_DIR` is set and the system starts
- **THEN** every file location resolves inside that directory exactly as if
  it were `SEBAS_HOME`
- **AND** the startup log carries a warning naming `SEBAS_HOME` as the
  canonical variable

#### Scenario: both variables set — home wins

- **WHEN** `SEBAS_HOME` and `SEBAS_STATE_DIR` are both set to different
  directories
- **THEN** every file location resolves inside `SEBAS_HOME`
- **AND** the startup log warns about the conflict

#### Scenario: a socket leaves XDG_RUNTIME_DIR

- **WHEN** `XDG_RUNTIME_DIR` is set or unset and the channel socket is
  created with no explicit override
- **THEN** the socket is created at `<sebas home>/run/core.sock`
- **AND** no path under `XDG_RUNTIME_DIR` or a per-uid temporary directory
  is consulted
