## ADDED Requirements

### Requirement: ACP resume loads by the real ACP session id

The generic ACP driver SHALL accept, in a load request, an explicit ACP session id that differs from the routing session id: when the caller provides the agent's real session id, the driver SHALL issue `session/load` with that id. The driver SHALL report the agent's real ACP session id (from `session/new` or the loaded conversation) back to the caller so the mapping can be persisted. When the load is rejected (unknown id or no load capability), the existing fresh-session fallback with `resumed = false` SHALL apply.

#### Scenario: Load uses the provided ACP session id

- **WHEN** a resume caller supplies an ACP session id distinct from the routing id
- **THEN** the driver issues `session/load` with the provided ACP id
- **AND** on success the routing id is unchanged and `resumed` is `true`

#### Scenario: Fresh spawn reports the agent session id

- **WHEN** a fresh ACP session is created
- **THEN** the spawn outcome carries the agent's real ACP session id (from `session/new`)
- **AND** the routing id maps to it for future resumes

#### Scenario: Rejected real-id load falls back to fresh

- **WHEN** the agent rejects `session/load` for the provided ACP session id
- **THEN** a fresh session starts with a new routing id
- **AND** `resumed` is `false`