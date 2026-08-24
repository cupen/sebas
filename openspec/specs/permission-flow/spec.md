# permission-flow Specification

## Purpose
Governs the full round-trip of a Claude tool-permission request: from the agent's PreToolUse hook, through a Feishu interactive card with Allow-once / Allow-session / Deny buttons, back to the hook response that unblocks the tool call. Defines how per-chat allowlists suppress repeat prompts and how stale clicks, session ends, and unanswered requests stay fail-closed.

## Requirements

### Requirement: Hook-driven permission request

The system SHALL surface every Claude PreToolUse hook invocation as a permission decision point keyed by the hook control `request_id` (which equals the Claude `tool_use_id`). The system SHALL park the hook callback until a decision is returned, and SHALL correlate the decision to the request strictly by `request_id`, never by position or arrival order.

#### Scenario: First-time tool call emits a permission card

- **WHEN** the agent invokes a tool whose `(tool, args)` signature is not on the current chat's allowlist
- **THEN** the system emits a `PermissionRequest` event carrying the session id, the `request_id`, the tool name, and the tool arguments
- **AND** the router sends a Feishu interactive card with three buttons: `Allow once`, `Allow session`, `Deny`
- **AND** the card is recorded in a `perm_cards` map keyed by `request_id` so a later button click can be correlated

#### Scenario: Parallel tool calls each get their own request id

- **WHEN** the agent invokes multiple tools concurrently
- **THEN** each PreToolUse hook callback parks an independent oneshot under its own `request_id`
- **AND** replies to one request do not resolve any other request

### Requirement: Three decision outcomes

The system SHALL support three user decisions on a permission card: `Allow once`, `Allow session`, and `Deny`. Each decision maps to a distinct hook output and a distinct post-click card state.

#### Scenario: Allow once approves this call only

- **WHEN** the user clicks `Allow once`
- **THEN** the hook callback returns `permissionDecision: allow` for this `request_id`
- **AND** the `(tool, args)` signature is NOT added to the allowlist
- **AND** the card flips in place to a resolved "已允许（仅本次）" state

#### Scenario: Allow session approves and remembers

- **WHEN** the user clicks `Allow session`
- **THEN** the hook callback returns `permissionDecision: allow`
- **AND** the exact `(tool, args)` signature is added to the per-chat allowlist
- **AND** the card flips in place to a resolved "已允许（本会话）" state

#### Scenario: Deny rejects the call

- **WHEN** the user clicks `Deny`
- **THEN** the hook callback returns `permissionDecision: deny`
- **AND** the allowlist is not modified
- **AND** the card flips in place to a resolved "已拒绝" state

### Requirement: Auto-approve on allowlist hit

The system SHALL skip the interactive card entirely when the `(tool, args)` signature is already present on the current chat's allowlist, and SHALL immediately resolve the hook callback with `allow`.

#### Scenario: Allowlisted signature runs silently

- **WHEN** the agent invokes a tool whose exact `(tool, args)` signature is on the current chat's allowlist
- **THEN** no permission card is sent
- **AND** the hook callback returns `permissionDecision: allow` without user interaction
- **AND** the tool runs immediately

#### Scenario: Slightly different args are not auto-approved

- **WHEN** the agent invokes a tool whose signature differs in any argument from every allowlisted entry
- **THEN** the request is treated as a miss and the normal card flow runs

### Requirement: Allowlist scope and lifetime

The allowlist SHALL be scoped to the current `SessionKey` (chat + thread) and SHALL be cleared when the session ends (terminal error, `/new`, or daemon restart with no resume).

#### Scenario: Session end wipes the allowlist

- **WHEN** a session terminates for any reason
- **THEN** the allowlist for that chat key is cleared
- **AND** the next session in the same chat starts with an empty allowlist

#### Scenario: /new resets permissions

- **WHEN** the user issues `/new` in a chat
- **THEN** the previous session's allowlist is discarded
- **AND** the first tool call in the new session prompts again

### Requirement: Stale click handling

The system SHALL distinguish a live permission card from an already-resolved one. A click on a resolved card SHALL NOT resolve any hook and SHALL surface a "请求已过期" notice to the user.

#### Scenario: Second click on a resolved card

- **WHEN** the user clicks a button on a permission card whose `request_id` has already been consumed
- **THEN** the system sends a new "⚠ 请求已过期" card
- **AND** no `PermissionReply` is emitted for that `request_id`

### Requirement: Fail-closed on missing responder

The system SHALL default to `deny` whenever a permission request cannot be answered — including when the router is unreachable, the session is gone, or a reply arrives for an unknown `request_id`.

#### Scenario: Reply for unknown request_id is dropped

- **WHEN** a `PermissionReply` arrives for a `request_id` that has no parked responder
- **THEN** the reply is logged and dropped
- **AND** the hook callback (if still pending elsewhere) resolves to `deny` via its own drop path

#### Scenario: Session termination while awaiting click

- **WHEN** the owning session terminates while a permission card is still awaiting user click
- **THEN** the parked oneshot is dropped
- **AND** the hook callback resolves to `deny`
- **AND** the tool call does not execute
