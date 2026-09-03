# agent-driver Specification

## Purpose

把 sebas 的三方 coding-agent 接入从 Claude Code 单实现抽象成驱动层：`AgentDriver` trait + 两类实现——Claude 专用驱动（保留 `cc-agent-sdk`，换取 Claude 专有能力如 token 用量计数）与通用 ACP 驱动（用 `agent-client-protocol` v1 驱动任意原生 ACP agent）。下游 router/飞书/webui 只消费统一的 `AcpEvent`/`AcpCommand` 防腐层词表，因此逐个新增三方 agent 只改配置、不改代码。配置 schema 从 `acp.claude` 迁移到 `acp.agents.<kind>`，权限往返跨驱动统一进入 webui 审查卡。

## Requirements

### Requirement: AgentDriver abstraction with two implementations

The system SHALL define an `AgentDriver` trait that abstracts driving one third-party coding-agent subprocess: `spawn(config)` producing a session handle that streams `AcpEvent`s, accepts `AcpCommand`s, and cancels on demand. The system SHALL provide two implementations: a `ClaudeDriver` that keeps driving Claude Code through `cc-agent-sdk`, and an `AcpDriver` that spawns a native ACP agent (e.g. `gemini --acp`) and speaks the Agent Client Protocol v1 through the `agent-client-protocol` crate. Both implementations SHALL emit the same `AcpEvent`/`AcpCommand` vocabulary, so downstream consumers need no driver-specific branches.

#### Scenario: Both drivers present the same vocabulary

- **WHEN** the router consumes events from either the Claude driver or the ACP driver
- **THEN** it observes only `AcpEvent` variants (`TextDelta`/`ThinkingDelta`/`ToolStart`/`ToolProgress`/`ToolEnd`/`PermissionRequest`/`Finished`/`Error`/`UsageUpdate`)
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

### Requirement: Configuration shape with backward-compatible migration

The system SHALL change `AcpConfig` from `acp.claude.*` to `acp.agents.<kind>` plus a `default` key. The system SHALL, on load, migrate a configuration file that uses the legacy `acp.claude` block into `acp.agents.claude` and SHALL warn when migration happens. When `default` is absent and exactly one agent is configured, that agent SHALL be the implicit default.

#### Scenario: Existing claude-only config keeps working

- **WHEN** the TOML config has `[acp.claude]` and no `agents` block
- **THEN** the loader treats that block as `agents.claude`, sets `default` to `claude`, and the runtime spawns Claude Code exactly as before
- **AND** the migration is announced with a single warning

#### Scenario: Bare default resolves to the sole configured agent

- **WHEN** `default` is absent and only one agent is configured
- **THEN** a session created with the bare `acp` hint resolves to that agent

### Requirement: Cross-driver permission routing through the webui review card

The system SHALL route permission requests from every driver through the same downstream channel, so a permission request raised by either the Claude driver or the ACP driver SHALL be addressable through the webui review card with the same `allow_once` / `allow_session` / `deny` / `escalate` decision vocabulary. The system SHALL names, in the `PermissionRequest` the driver emits, the `request_id` as `<kind-slug>:<raw-id>` so ids from different drivers cannot collide, and SHALL decode it back to the raw id when delivering the answer to the owning driver.

#### Scenario: Permission round-trip works for an ACP agent

- **WHEN** a native-ACP agent raises a permission request for a tool the policy gates
- **THEN** the webui shows the review card with the same four actions
- **AND** the chosen decision is delivered to the ACP driver, which answers the ACP permission with the mapped `PermissionOption.kind`
- **AND** the request id carries the kind slug so it is unambiguous across sessions

#### Scenario: Claude permission reaches the webui (gap fix)

- **WHEN** a Claude Code session raises a `PermissionRequest`
- **THEN** the `InProcessBackend` (the ACP-path session backend) forwards it to the webui as a `PermissionNotice`
- **AND** `answer_permission` on that backend delivers the decision back to the session, instead of returning the trait default `false`

### Requirement: Honest reachability reporting per kind

The system SHALL provide a `sebas agent-kinds list` command that reports, for each configured agent, its kind slug, reachability, version when available, and a failure cause when unreachable. The reachability check SHALL probe the configured command's presence and its ability to report a version. The webui create-session form SHALL expose one entry per reachable agent plus a `native` entry, and SHALL omit unreachable agents from the dropdown.

#### Scenario: Reachability distinguishes present from absent

- **WHEN** `sebas agent-kinds list` runs and one configured agent's command is not on `PATH`
- **THEN** that agent reports `reachable=false` with a cause string, while present agents report `reachable=true`
- **AND** the webui dropdown lists only reachable agents

#### Scenario: Unsupported driver tag fails fast

- **WHEN** a configuration declares `driver = "foobar"` for an agent
- **THEN** the loader returns a configuration error naming the unsupported driver and refuses to start