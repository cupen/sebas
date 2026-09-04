## Purpose

Owns the mapping between sebas's routing session id and the real ACP session id that the agent process knows. ACP agents (opencode, gemini, …) address a conversation by their own session id, which differs from sebas's routable uuid; persisting this mapping is what makes resume actually restore the conversation instead of silently starting fresh.

## ADDED Requirements

### Requirement: Routing id ↔ ACP session id mapping

The system SHALL record, for each ACP-backed session, a mapping from the sebas routing session id to the agent's real ACP session id. The mapping SHALL be established when the session is created (fresh or resumed) and SHALL be persisted with the session record so it survives a daemon restart. Sessions that do not expose a distinct ACP session id (e.g. Claude's dedicated driver, where the conversation id equals the routing id) SHALL record no mapping and resume as today.

#### Scenario: Fresh ACP spawn records the mapping

- **WHEN** a native-ACP session is spawned fresh and the agent returns its own session id
- **THEN** the routing session id is mapped to the agent's real ACP session id
- **AND** the mapping is persisted with the session record

#### Scenario: Resume reads the mapping

- **WHEN** a persisted session is resumed after a restart
- **THEN** the driver loads the conversation by the recorded ACP session id, not by the routing id
- **AND** a successful load reports `resumed = true` and keeps the routing id

### Requirement: Missing mapping falls back honestly

When a session record has no ACP session id (legacy records, agents without a distinct id, or a lost mapping), a resume attempt SHALL fall back to a fresh session with a new routing id and report `resumed = false`, exactly as if the load had been rejected. It MUST NOT fabricate a mapping or guess an id.

#### Scenario: Resume with no mapping starts fresh

- **WHEN** a session with no recorded ACP session id is resumed
- **THEN** a fresh session starts with a new routing id
- **AND** `resumed` is `false` so the caller can inform the user the old conversation is gone

#### Scenario: Load failure keeps the old mapping intact

- **WHEN** a resume attempts to load a conversation and the agent rejects it
- **THEN** the session falls back to fresh with a new routing id
- **AND** the original routing-id ↔ ACP-id mapping is left unchanged in storage (the old conversation is still addressable by future loads)