## ADDED Requirements

### Requirement: ACP session resume through session/load

The generic ACP driver SHALL implement resume for native ACP agents: when a session starts with a load request, the driver SHALL issue the ACP `session/load` request for the persisted session id and continue that conversation. When the agent declines or cannot load the session (unknown id, or the agent does not advertise load capability), the driver SHALL fall back to a fresh session with a new id and report `resumed = false`. A fallback SHALL NOT surface a raw protocol error to the caller.

#### Scenario: ACP resume continues the loaded conversation

- **WHEN** a native ACP agent is spawned in load mode with a persisted session id and the agent supports `session/load`
- **THEN** the driver issues a load for that id, the agent streams the resumed conversation in later prompts, and the spawn reports `resumed = true` with the same routing id

#### Scenario: ACP resume falls back to fresh when the session is gone

- **WHEN** the ACP agent responds to `session/load` with an error (conversation not found)
- **THEN** the driver starts a fresh session with a newly minted id
- **AND** the spawn reports `resumed = false` so the caller can tell the user the old conversation is gone

#### Scenario: Resume is rejected, not faked, for agents without load support

- **WHEN** the driver is asked to load a session but the ACP agent does not support load
- **THEN** resume is handled honestly: the fresh-session fallback applies (never a fabricated successful resume)
- **AND** the caller is informed via `resumed = false`