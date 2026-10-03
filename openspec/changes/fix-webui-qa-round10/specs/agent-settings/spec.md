## ADDED Requirements

### Requirement: Store-row reachability probe honors absolute paths

The agent catalog probe SHALL treat an agent command whose first element is an absolute path (forward- or backslash-separated) as a direct file-existence check, identically for config-seeded rows and Settings-store rows. A store-row agent whose binary exists and is spawnable SHALL be reported reachable, and SHALL be selectable in the new-session dialog without a restart. The probe MUST NOT report `command not found` for a path that exists on disk.

#### Scenario: GUI-created agent with an absolute Windows path is reachable

- **WHEN** an operator creates an agent in Settings → Agents with the binary path `C:/<repo>/target/debug/fake-claude.exe` (or the backslash form), where the file exists
- **THEN** the catalog row shows the reachable badge (no `command not found` cause)
- **AND** the agent appears enabled in the new-session dialog's agent dropdown immediately, without a core restart

#### Scenario: Unreachable guidance names the failing surface

- **WHEN** an agent is reported unreachable because its binary cannot be resolved
- **THEN** the new-session dialog's guidance for that agent directs the operator to Settings → Agents (the binary path surface)
- **AND** only a model/credential-caused unavailability (native kernel without provider credentials) directs to Settings → Models

### Requirement: Duplicate agent id is rejected with one consistent message

The agent creation form SHALL present exactly one outcome for an already-existing agent id: the save is rejected with a single visible error stating the id exists. The form MUST NOT simultaneously promise an overwrite (warning) while rejecting the save (error).

#### Scenario: Creating an agent with an existing id

- **WHEN** the operator submits the new-agent form with an id that already exists in the store
- **THEN** the save is rejected with one visible error naming the duplicate id
- **AND** no warning promising an overwrite is shown alongside the rejection
