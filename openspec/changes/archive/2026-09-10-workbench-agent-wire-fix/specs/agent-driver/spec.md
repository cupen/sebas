## ADDED Requirements

### Requirement: Driver is a configuration-layer concept, not wire vocabulary

The driver (`claude` vs `acp`) that a configured agent uses SHALL remain a configuration concern, resolved server-side from the agent's config entry. The wire vocabulary for selecting an agent SHALL use the agent's configured name (id) directly, with no driver prefix or namespace; clients SHALL NOT need to know which driver backs a given agent. The `driver` field SHALL NOT appear in any API response payload.

#### Scenario: wire uses agent id, not driver name

- **WHEN** a client creates a session for the agent configured as `[acp.agents.claudecode]` (driver `claude`)
- **THEN** the request carries `agent = "claudecode"`, not `"claude"` or `"acp:claudecode"`

#### Scenario: driver does not leak into API payloads

- **WHEN** `GET /api/agents` is returned for a config mixing `driver = "claude"` and `driver = "acp"` agents
- **THEN** no entry in the payload contains a `driver` field, and all entries use the same shape

#### Scenario: server resolves driver from agent id

- **WHEN** the backend receives `agent = "codex"` where `[acp.agents.codex] driver = "acp"`
- **THEN** the session spawns through the ACP driver without the client having named the driver

## MODIFIED Requirements

### Requirement: Honest reachability reporting per kind

The system SHALL provide a `sebas agent-kinds list` command and a webui agent catalog endpoint that report, for each configured agent, its id, display name, reachability, version when available, and a failure cause when unreachable. The reachability check SHALL probe the configured command's presence and its ability to report a version. The webui create-session form SHALL expose one entry per reachable agent plus a `native` entry for the built-in kernel, and SHALL mark unreachable agents as unavailable with their cause. The agent catalog endpoint SHALL be the single source of truth for agent availability on the frontend.

#### Scenario: Reachability distinguishes present from absent

- **WHEN** `sebas agent-kinds list` runs and one configured agent's command is not on `PATH`
- **THEN** that agent reports `reachable=false` with a cause string, while present agents report `reachable=true`
- **AND** the webui dropdown lists only reachable agents

#### Scenario: native kernel appears in the catalog

- **WHEN** the webui requests the agent catalog and the native kernel lacks provider credentials
- **THEN** the catalog includes a `native` entry with `reachable = false` and a cause, and the webui marks the native option unavailable with that cause

#### Scenario: catalog is the single availability source

- **WHEN** the webui composer renders its agent selector
- **THEN** the available/unavailable state and causes come from the agent catalog endpoint, not from a separate per-execution-body report

#### Scenario: Unsupported driver tag fails fast

- **WHEN** a configuration declares `driver = "foobar"` for an agent
- **THEN** the loader returns a configuration error naming the unsupported driver and refuses to start
