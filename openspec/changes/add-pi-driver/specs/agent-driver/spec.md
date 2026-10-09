## RENAMED Requirements

- FROM: `### Requirement: AgentDriver abstraction with two implementations`
- TO: `### Requirement: AgentDriver abstraction with three implementations`

## MODIFIED Requirements

### Requirement: AgentDriver abstraction with three implementations

The system SHALL define an `AgentDriver` trait that abstracts driving one third-party coding-agent subprocess: `spawn(config)` producing a session handle that streams `AcpEvent`s, accepts `AcpCommand`s, and cancels on demand. The system SHALL provide three implementations: a `ClaudeDriver` that keeps driving Claude Code through `cc-agent-sdk`, an `AcpDriver` that spawns a native ACP agent (e.g. `gemini --acp`) and speaks the Agent Client Protocol v1 through the `agent-client-protocol` crate, and a `PiDriver` that spawns `pi --mode rpc` and speaks pi's headless RPC protocol (stdio JSONL) — pi 官方无 ACP 支持（讨论 #4444 未立项），故不走通用 ACP 驱动。All three implementations SHALL emit the same `AcpEvent`/`AcpCommand` vocabulary, so downstream consumers need no driver-specific branches.

#### Scenario: Both drivers present the same vocabulary

- **WHEN** the router consumes events from any of the Claude, ACP, or Pi drivers
- **THEN** it observes only `AcpEvent` variants (`TextDelta`/`ThinkingDelta`/`ToolStart`/`ToolProgress`/`ToolEnd`/`PermissionRequest`/`Finished`/`Error`/`UsageUpdate`/`ModelChanged`/`ModeChanged`/`AvailableCommands`)
- **AND** no `agent-client-protocol`, `cc-agent-sdk`, or pi-protocol type leaks past the driver module boundary

#### Scenario: Claude driver preserves usage accounting

- **WHEN** Claude Code streams a result carrying `cache_read_input_tokens`
- **THEN** the Claude driver emits an `AcpEvent::UsageUpdate` carrying that count, which a generic ACP driver does not emit for agents that lack it

#### Scenario: ACP driver spawns a native ACP agent

- **WHEN** an agent is configured with `driver = "acp"` and a `command` such as `gemini --acp`
- **THEN** the ACP driver spawns that command as a subprocess, negotiates ACP v1 `initialize`, and streams its `session/update` events translated into `AcpEvent`s

#### Scenario: Pi driver spawns pi in RPC mode

- **WHEN** an agent is configured with `driver = "pi"` and a binary path resolving to the pi CLI
- **THEN** the Pi driver spawns `pi --mode rpc` as a subprocess, correlates JSONL commands and responses, and translates pi's session events into `AcpEvent`s (see `pi-agent` capability for the pi-specific contract)

## ADDED Requirements

### Requirement: Product default agent is pi

默认 agent 的解析顺序 SHALL 为：显式 `[acp] default` 永远优先；无显式 default 且恰好只配一个 agent 时，该 agent 为隐式默认（既有规则不变）；**配了多个 agent 且无显式 default 时**，SHALL 取优先链 `pi` → `claude` 中**已配置**者，两者都未配置则取已配置 agent 中字典序最小者（确定性；绝不留下未设 default 的多 agent 配置——否则启动会去探测未配置的 `pi` 二进制而失败）；一个 agent 都未配置时，回退探测 SHALL 依次尝试 `pi` 与 `claude` 二进制（以可解析为准，命中者写入启动日志，两者皆缺时按既有口径失败）。随附的示例配置 SHALL 种子 `pi`（默认）与 `claude`（保留可切回）两个条目并显式声明 `default = "pi"`。缺省/隐式解析出的默认 agent 若二进制不可达，SHALL 在目录与 spawn 处如实报不可达及原因，不得静默改选其它 agent。

#### Scenario: fresh example config defaults to pi

- **WHEN** 以随附示例配置启动且未改动 `[acp]` 段
- **THEN** 默认 agent 解析为 `pi`，且 `claude` 条目仍在目录中可选

#### Scenario: explicit default still wins

- **WHEN** 配置声明 `[acp] default = "claude"`（或任何已配置 agent 的 id）
- **THEN** 默认 agent 为该 id，回退探测不参与

#### Scenario: multi-agent config without a default resolves deterministically

- **WHEN** 配置了多个 agent 且未设 `[acp] default`（例如 `claude` + 一个自定义 ACP agent，未配置 `pi`）
- **THEN** 默认 agent 取优先链中已配置者（此处 `claude`），启动不因探测未配置的 `pi` 二进制而失败
- **AND** 若 `pi` 也在已配置之列，`pi` 优先（产品默认决策）

#### Scenario: zero-agent fallback probes pi then claude

- **WHEN** 配置中没有任何 `[acp.agents.*]` 条目且未设 `default`
- **THEN** 回退按 `pi` → `claude` 的顺序探测二进制，命中者为回退默认并写入启动日志
- **AND** 两者皆不可解析时按既有失败口径拒绝，不静默放行

#### Scenario: default agent absent is reported honestly

- **WHEN** 默认 agent（如 pi）二进制不在 PATH
- **THEN** agent 目录将其报为不可达并带原因，未显式指定 agent 的新会话创建失败并如实点名缺失
- **AND** 系统不替操作员静默改用其它 agent
