## MODIFIED Requirements

### Requirement: AgentDriver abstraction with two implementations

The system SHALL define an `AgentDriver` trait that abstracts driving one third-party coding-agent subprocess: `spawn(config)` producing a session handle that streams `AcpEvent`s, accepts `AcpCommand`s, and cancels on demand. The trait is the **abstraction/policy layer**: it owns which driver a configured agent kind resolves to, the open kind registry, cross-driver permission routing, and honest reachability reporting (below) — it does NOT own the per-session subprocess lifecycle or the ACP protocol details, which belong to the `acp-driver` capability (the runtime layer for ACP children). The system SHALL provide two implementations: a `ClaudeDriver` that keeps driving Claude Code through `cc-agent-sdk`, and an `AcpDriver` that drives a native ACP agent through the ACP subprocess runtime (`acp-driver`). Both implementations SHALL emit the same `AcpEvent`/`AcpCommand` vocabulary, so downstream consumers need no driver-specific branches.

#### Scenario: Both drivers present the same vocabulary

- **WHEN** the router consumes events from either the Claude driver or the ACP driver
- **THEN** it observes only `AcpEvent` variants (`TextDelta`/`ThinkingDelta`/`ToolStart`/`ToolProgress`/`ToolEnd`/`PermissionRequest`/`Finished`/`Error`/`UsageUpdate`)
- **AND** no `agent-client-protocol` or `cc-agent-sdk` type leaks past the driver module boundary

#### Scenario: Claude driver preserves usage accounting

- **WHEN** Claude Code streams a result carrying `cache_read_input_tokens`
- **THEN** the Claude driver emits an `AcpEvent::UsageUpdate` carrying that count, which a generic ACP driver does not emit for agents that lack it

#### Scenario: ACP driver spawns a native ACP agent

- **WHEN** an agent is configured with `driver = "acp"` and a `command` such as `gemini --acp`
- **THEN** the ACP driver resolves the kind to the ACP subprocess runtime (`acp-driver`), which spawns that command, negotiates ACP v1 `initialize`, and streams its `session/update` events translated into `AcpEvent`s
