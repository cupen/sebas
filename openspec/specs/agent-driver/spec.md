# agent-driver Specification

## Purpose

把 sebas 的三方 coding-agent 接入从 Claude Code 单实现抽象成驱动层：`AgentDriver` trait + 两类实现——Claude 专用驱动（保留 `cc-agent-sdk`，换取 Claude 专有能力如 token 用量计数）与通用 ACP 驱动（用 `agent-client-protocol` v1 驱动任意原生 ACP agent）。下游 router/飞书/webui 只消费统一的 `AcpEvent`/`AcpCommand` 防腐层词表，因此逐个新增三方 agent 只改配置、不改代码。权限往返跨驱动统一进入 webui 审查卡。

## Requirements

### Requirement: AgentDriver abstraction with two implementations

The system SHALL define an `AgentDriver` trait that abstracts driving one third-party coding-agent subprocess: `spawn(config)` producing a session handle that streams `AcpEvent`s, accepts `AcpCommand`s, and cancels on demand. The system SHALL provide two implementations: a `ClaudeDriver` that keeps driving Claude Code through `cc-agent-sdk`, and an `AcpDriver` that spawns a native ACP agent (e.g. `gemini --acp`) and speaks the Agent Client Protocol v1 through the `agent-client-protocol` crate. Both implementations SHALL emit the same `AcpEvent`/`AcpCommand` vocabulary, so downstream consumers need no driver-specific branches.

#### Scenario: Both drivers present the same vocabulary

- **WHEN** the router consumes events from either the Claude driver or the ACP driver
- **THEN** it observes only `AcpEvent` variants (`TextDelta`/`ThinkingDelta`/`ToolStart`/`ToolProgress`/`ToolEnd`/`PermissionRequest`/`Finished`/`Error`/`UsageUpdate`/`ModelChanged`)
- **AND** no `agent-client-protocol` or `cc-agent-sdk` type leaks past the driver module boundary

#### Scenario: Claude driver preserves usage accounting

- **WHEN** Claude Code streams a result carrying `cache_read_input_tokens`
- **THEN** the Claude driver emits an `AcpEvent::UsageUpdate` carrying that count, which a generic ACP driver does not emit for agents that lack it

#### Scenario: ACP driver spawns a native ACP agent

- **WHEN** an agent is configured with `driver = "acp"` and a `command` such as `gemini --acp`
- **THEN** the ACP driver spawns that command as a subprocess, negotiates ACP v1 `initialize`, and streams its `session/update` events translated into `AcpEvent`s

### Requirement: Open agent registry keyed by kind, not a closed enum

The system SHALL key configured agents by an open `kind` slug (a string), not by a closed Rust enum. Each configured agent SHALL declare its driver via a serde tag (`driver = "claude"` for the dedicated Claude driver, `driver = "acp"` for the generic ACP driver). Adding a new native-ACP agent SHALL require only a new `agents.<slug>` entry, with no code change or recompile.

#### Scenario: Adding a native ACP agent is configuration-only

- **WHEN** the user adds `[acp.agents.cursor] driver = "acp", command = ["cursor-agent", "acp"]`
- **THEN** `sebas agent-kinds list` reports `cursor` as reachable when its binary is on `PATH`
- **AND** no Rust code is changed or recompiled

#### Scenario: ACP agent reaches the same session surface as Claude

- **WHEN** a session is created with a backend hint selecting an ACP agent
- **THEN** the spawned session accepts prompts, streams text and tool events, and answers cancellation exactly like a Claude session

### Requirement: Legacy `[acp.claude]` block is rejected

Configurations using the legacy `[acp.claude]` table SHALL be rejected at parse time. The TOML deserializer reports the offending `[acp.claude]` line; the loader does not rewrite the block into `[acp.agents.claude]` and does not pick a default on the user's behalf. Configurations that declare only `[acp.agents.<kind>]` tables (with `default` either set explicitly or implicit when exactly one agent is configured) continue to load as today.

#### Scenario: Legacy `[acp.claude]` block fails parse

- **WHEN** the TOML config contains `[acp.claude]`
- **THEN** parsing fails with an error naming the `[acp.claude]` line and the loader does not produce a `Config`

#### Scenario: New `[acp.agents.claude]` block loads

- **WHEN** the TOML config contains
  `[acp.agents.claude] driver = "claude" path = "…" args = […]`
- **THEN** parsing succeeds and `cfg.acp.agents["claude"]` is set
- **AND** with no other agent configured and no `default` set, `cfg.acp.default`
  resolves to `"claude"` (implicit single-agent default)

### Requirement: Cross-driver permission routing through the webui review card

The system SHALL route permission requests from every driver through the same downstream channel, so a permission request raised by either the Claude driver or the ACP driver SHALL be addressable through the webui review card. The full decision vocabulary is `allow_once` / `allow_session` / `deny` / `escalate`. The `escalate` decision (a one-shot allow carrying the operator's reason) is meaningful only for the native kernel; when the owning execution body is an ACP driver, an `escalate` decision SHALL be delivered as `allow_once` and the downgrade SHALL be logged. The system SHALL name, in the `PermissionRequest` the driver emits, the `request_id` as `<kind-slug>:<raw-id>` so ids from different drivers cannot collide, and SHALL decode it back to the raw id when delivering the answer to the owning driver.

#### Scenario: Permission round-trip works for an ACP agent

- **WHEN** a native-ACP agent raises a permission request for a tool the policy gates
- **THEN** the webui shows the review card
- **AND** the chosen decision is delivered to the ACP driver, which answers the ACP permission with the mapped `PermissionOption.kind`
- **AND** the request id carries the kind slug so it is unambiguous across sessions

#### Scenario: escalate falls back to allow-once for an ACP agent

- **WHEN** the operator answers `escalate` on a permission request whose owning execution body is an ACP driver
- **THEN** the decision delivered to that ACP driver is `allow_once` (ACP has no escalate equivalent)
- **AND** the downgrade is logged

#### Scenario: Claude permission reaches the webui (gap fix)

- **WHEN** a Claude Code session raises a `PermissionRequest`
- **THEN** the `InProcessBackend` (the ACP-path session backend) forwards it to the webui as a `PermissionNotice`
- **AND** `answer_permission` on that backend delivers the decision back to the session, instead of returning the trait default `false`

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
