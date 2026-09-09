## MODIFIED Requirements

### Requirement: One subprocess per session

The system SHALL spawn exactly one Claude Code child process per sebas session — the ACP subprocess runtime layer owned by this capability, which the `agent-driver` abstraction layer's `AcpDriver` implementation and the router consume. The sebas routing id SHALL be the same value as the Claude conversation id. Driver-kind resolution (which driver a configured agent resolves to), the open kind registry, and cross-driver permission routing belong to `agent-driver`, not here.

#### Scenario: Fresh spawn mints a new id

- **WHEN** the manager is asked to create a new session
- **THEN** a fresh UUID is minted
- **AND** the child is launched with `--session-id <uuid>` so the Claude conversation id equals the routing id

#### Scenario: Resume reuses the existing id

- **WHEN** the manager is asked to resume a previously persisted session id
- **THEN** the child is launched with `--resume <id>` only
- **AND** `--session-id` is NOT passed (the real CLI rejects that combination)
- **AND** the routing id remains the resumed conversation id

#### Scenario: Runtime serves any driver kind

- **WHEN** the abstraction layer resolves an agent kind to the ACP driver
- **THEN** the runtime owns spawn, resume, streaming, and cancellation for that kind's child per the requirements below
