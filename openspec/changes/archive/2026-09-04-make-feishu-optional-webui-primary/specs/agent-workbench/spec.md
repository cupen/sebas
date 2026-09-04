## ADDED Requirements

### Requirement: Session execution over the native agent kernel

The system SHALL support routing a session's execution to the native agent kernel (`sebas-agent`) instead of the Claude Code ACP bridge. A session SHALL carry an execution-body hint: `native` (native kernel) or `acp` (ACP bridge, the default). On `native`, the kernel SHALL own the session — session spawning, the turn loop, the tool set (bash / read / write / edit / glob / grep), agent configuration injection (`AGENTS.md` / `CLAUDE.md`), and cancellation/budget semantics. On `acp`, execution proceeds through the ACP child as today.

#### Scenario: WebUI session on the native kernel

- **WHEN** a WebUI create-session request carries `backend = "native"`
- **THEN** the session is created under an `agent-*` session key and its turns/tools run inside the native kernel
- **AND** the session appears in the WebUI snapshot with project_dir honored and zero turns until the first prompt

#### Scenario: Feishu session on the native kernel

- **WHEN** feishu is enabled and a feishu inbound message routes to a `native`-executed session
- **THEN** the session exists in the same shared snapshot visible to the WebUI
- **AND** its tool traces and completion text are readable via the WebUI turn-content API

#### Scenario: Default execution body stays ACP

- **WHEN** no execution-body hint is present (feishu or WebUI default)
- **THEN** the session executes on the ACP bridge as today, preserving behavior

### Requirement: Gated call approval on the native kernel

The native kernel SHALL surface gated tool calls (bash / write / edit / apply_patch in Ask mode) as approval requests rather than executing them. When the WebUI is present, the approval SHALL be presented through the WebUI review-card channel; the operator's decision (allow-once / allow-session / deny with reason) SHALL round-trip to the kernel's approver. Failure to answer SHALL fail closed (the call is not executed).

#### Scenario: WebUI answers a native gated call

- **WHEN** a native session requests approval for a gated tool call
- **THEN** the WebUI review card presents it with decision options
- **AND** an allow-once decision lets only that call through, a deny rejects it, and no answer leaves it unexecuted

#### Scenario: No WebUI attached to a native gated call

- **WHEN** a native session requests approval but no WebUI consumer is attached
- **THEN** the request is denied (fail-closed) and the tool call is not executed