## MODIFIED Requirements


### Requirement: Hook-driven permission request

The system SHALL surface every Claude PreToolUse hook invocation as a permission decision point keyed by the hook control `request_id` (which equals the Claude `tool_use_id`). The system SHALL park the hook callback until a decision is returned, and SHALL correlate the decision to the request strictly by `request_id`, never by position or arrival order.

This SHALL hold for every turn of a session, not only the first: a session that has already decided at least one approval in an earlier turn SHALL register subsequent tool approvals exactly like the first — the pending request SHALL appear in the per-session approvals read model and SHALL render an approval card in every connected workbench client without a page reload. A turn whose approval request fails to register SHALL NOT remain silently parked: it MUST be observable as pending (read model + rail waiting badge) or must not park at all; an unregistered-but-parked turn that only cancel can recover is a violation.

#### Scenario: First-time tool call emits a permission card

- **WHEN** the agent invokes a tool whose `(tool, args)` signature is not on the current chat's allowlist
- **THEN** the system emits a `PermissionRequest` event carrying the session id, the `request_id`, the tool name, and the tool arguments
- **AND** the router sends a Feishu interactive card with three buttons: `Allow once`, `Allow session`, `Deny`
- **AND** the card is recorded in a `perm_cards` map keyed by `request_id` so a later button click can be correlated

#### Scenario: Parallel tool calls each get their own request id

- **WHEN** the agent invokes multiple tools concurrently
- **THEN** each PreToolUse hook callback parks an independent oneshot under its own `request_id`
- **AND** replies to one request do not resolve any other request

#### Scenario: 每一轮工具环的审批请求都到达审批面

- **WHEN** a session in ask mode completes a decided approval turn (allow or deny) and a later turn in the same session requests another tool approval
- **THEN** the new pending request appears in the session approvals read model and an approval card renders in the workbench without a page reload

#### Scenario: 第二轮审批不丢（进程级）

- **WHEN** a fake-claude session driven through two `perm` turns (first decided via the API, second observed read-only)
- **THEN** the approvals read model lists the second pending request while the turn waits, and the session is decidable through the same surface as the first
