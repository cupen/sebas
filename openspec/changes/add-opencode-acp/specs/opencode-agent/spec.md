## Purpose

Makes opencode (sst/opencode, `opencode acp` subcommand) a supported agent kind in sebas: users configure it under `[acp.agents.opencode]` with the generic ACP driver and get the full session surface (prompt, streaming text/tool deltas, permission round-trip, resume) without any code change. It also anchors the ACP resume capability: opencode's ACP `loadSession` support is the first real consumer of the generic `session/load` path in `acp-driver`.

## ADDED Requirements

### Requirement: opencode is reachable as an ACP agent

The system SHALL treat `opencode acp` as a first-class ACP agent when configured as `driver = "acp"`, `command = ["opencode", "acp"]`. Reachability probing and the create-session surface SHALL work for it just as for any other agent kind.

#### Scenario: Configured opencode appears in agent discovery

- **WHEN** the config has `[acp.agents.opencode] driver = "acp", command = ["opencode", "acp"]` and `opencode` is on `PATH`
- **THEN** `sebas agent-kinds list` reports `opencode` as reachable with a version string
- **AND** the webui create-session dropdown lists an opencode entry

#### Scenario: Missing opencode binary reports unreachable

- **WHEN** `opencode` is not on `PATH`
- **THEN** `agent-kinds list` reports `opencode reachable=false` with a cause, and the webui omits the entry

### Requirement: opencode sessions stream and answer permission requests

A session bound to the opencode kind SHALL behave like any other ACP session: prompts stream text and tool events through the standard `AcpEvent` vocabulary, permits raise the webui review card, and cancellation works.

#### Scenario: opencode session streams a turn

- **WHEN** a user prompts an opencode session
- **THEN** the session streams text deltas (and tool start/end events when tools run) to the same event surface as a Claude session

#### Scenario: opencode permission request reaches the webui

- **WHEN** opencode requests permission for a tool call
- **THEN** a `PermissionRequest` with a `opencode:<raw-id>` request id is raised
- **AND** an `allow_once` / `allow_session` / `deny` decision from the review card is delivered back to opencode

### Requirement: opencode session resume

A previously persisted opencode session SHALL resume through the ACP `session/load` path: the driver loads the existing conversation instead of starting fresh, so the resumed session continues the same conversation with the same routing id.

#### Scenario: Resume loads the existing opencode conversation

- **WHEN** a session with kind `opencode` is resumed with a persisted session id and the agent supports load
- **THEN** the new session continues the previous conversation with the same routing id
- **AND** `resumed` is `true` in the spawn outcome

#### Scenario: Resume fallback when the old conversation is gone

- **WHEN** the ACP agent cannot load the requested session id (unknown conversation or agent without load support)
- **THEN** a fresh session starts with a new id
- **AND** the spawn outcome reports `resumed = false` so the caller can inform the user