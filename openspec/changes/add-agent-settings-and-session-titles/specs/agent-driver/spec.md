## MODIFIED Requirements

### Requirement: Open agent registry keyed by kind, not a closed enum

The system SHALL key agents by an open `kind` slug (a string), not by a closed Rust enum. Each agent SHALL declare its driver via a serde tag (`driver = "claude"` for the dedicated Claude driver, `driver = "acp"` for the generic ACP driver). Adding a new native-ACP agent SHALL require only a new `agents.<slug>` config entry or an agents-store row created from Settings, with no code change or recompile. The spawn path SHALL resolve the requested kind dynamically: a kind absent from the config-built registry SHALL be resolved from the agents-store snapshot and spawned with the stored launch definition.

#### Scenario: Adding a native ACP agent is configuration-only

- **WHEN** the user adds `[acp.agents.cursor] driver = "acp", command = ["cursor-agent", "acp"]`
- **THEN** `sebas agent-kinds list` reports `cursor` as reachable when its binary is on `PATH`
- **AND** no Rust code is changed or recompiled

#### Scenario: Store-defined agent is spawnable without restart

- **WHEN** an agent row is created through the Settings surface while the processes are running
- **THEN** a session created with that kind spawns with the stored definition, with no restart

#### Scenario: ACP agent reaches the same session surface as Claude

- **WHEN** a session is created with a backend hint selecting an ACP agent
- **THEN** the spawned session accepts prompts, streams text and tool events, and answers cancellation exactly like a Claude session
