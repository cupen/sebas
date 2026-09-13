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

The system SHALL support three user decisions on a Feishu permission card: `Allow once`, `Allow session`, and `Deny`. This capability owns the Feishu-side rendering; the cross-driver decision vocabulary (including `escalate`) and `request_id` namespacing are governed by `agent-driver`. Each decision maps to a distinct hook output and a distinct post-click card state. `Allow session` SHALL mean: allow the current request AND switch the session's mode to `auto` — it is the Feishu-side mode-switching surface; the former chat-level grant-all allowlist is retired and mode gating is the only "stop asking" mechanism. `auto` SHALL leave an audit trail when selected from the card: the card SHALL flip in place to a "✅ 已切换自动模式" state.

#### Scenario: Allow once approves this call only

- **WHEN** the user clicks `Allow once`
- **THEN** the hook callback returns `permissionDecision: allow` for this `request_id`
- **AND** the session's mode is NOT changed
- **AND** the card flips in place to a resolved "已允许（仅本次）" state

#### Scenario: Allow session approves and remembers

- **WHEN** the user clicks `Allow session`（「本会话不再询问」）
- **THEN** the hook callback returns `permissionDecision: allow` for this `request_id`（当前请求立即放行）
- **AND** the session's mode is switched to `auto`（生效与 `desired`/`effective` 的既有语义一致）
- **AND** the card flips in place to the "✅ 已切换自动模式" audit-trail state

#### Scenario: Allow session with failed mode switch is honest

- **WHEN** the user clicks `Allow session` but the mode switch is rejected or cannot reach the execution body
- **THEN** the current request is still allowed（点击的首要语义不回退）
- **AND** the card presents the failure honestly（mode 未切换的如实状态），不伪装成已切换

#### Scenario: Deny rejects the call

- **WHEN** the user clicks `Deny`
- **THEN** the hook callback returns `permissionDecision: deny`
- **AND** the session's mode is not modified
- **AND** the card flips in place to a resolved "已拒绝" state

### Requirement: Stale click handling

The system SHALL distinguish a live permission card from an already-resolved one. A click on a resolved card SHALL NOT resolve any hook and SHALL surface a "请求已过期" notice to the user.

#### Scenario: Second click on a resolved card

- **WHEN** the user clicks a button on a permission card whose `request_id` has already been consumed
- **THEN** the system sends a new "⚠ 请求已过期" card
- **AND** no `PermissionReply` is emitted for that `request_id`

### Requirement: Fail-closed on missing responder

The system SHALL default to `deny` whenever a permission request cannot be answered — including when the router is unreachable, the session is gone, or a reply arrives for an unknown `request_id`. **补充（远程会话）**：control plane 暂时不可达 SHALL NOT 算作「无法应答」——远程会话的权限请求 SHALL 保持 parked，直至 control plane 返回或该请求所属会话终止（节点重启等），且 parked 期间 SHALL NOT 有任何本地裁决路径。

#### Scenario: Reply for unknown request_id is dropped

- **WHEN** a `PermissionReply` arrives for a `request_id` that has no parked responder
- **THEN** the reply is logged and dropped
- **AND** the hook callback (if still pending elsewhere) resolves to `deny` via its own drop path

#### Scenario: Session termination while awaiting click

- **WHEN** the owning session terminates while a permission card is still awaiting user click
- **THEN** the parked oneshot is dropped
- **AND** the hook callback resolves to `deny`
- **AND** the tool call does not execute

#### Scenario: unreachable control plane parks instead of denying

- **WHEN** a remote session's permission request is parked and the control plane becomes unreachable
- **THEN** the request stays parked and is not resolved to `deny` by the absence
- **AND** it is resolved when the control plane returns, or when its session terminates

### Requirement: Session mode gates whether a decision is requested

Each session SHALL carry a mode (`ask`, `edit`, `allow`, `auto`, and any further values the control plane defines) that decides whether a tool action needs a decision at all. Under `auto` the session SHALL run without producing permission requests. The mode SHALL be a desired value held by the control plane, and the execution side SHALL report the mode it actually enforces. An execution body that cannot enforce a mode SHALL report that fact rather than appearing to enforce it. `auto` SHALL NOT be the default mode and SHALL leave an audit trail when selected.

The mode SHALL be selectable at session creation time from the control-plane surfaces (web session create and mid-session mode switch), not only assigned by node-side defaults. On the local (in-process) claude execution path, the control-plane mode SHALL map onto the claude CLI's permission-mode vocabulary by convention — `ask` → CLI default (no flag), `edit` → acceptEdits, `allow`/`auto` → bypassPermissions — applied as a spawn-time flag and switchable at runtime; an execution body that receives a mode it cannot apply SHALL NOT fail the session (non-fatal, same posture as model selection). The node-link path SHALL carry the control-plane mode verbatim as its existing gate vocabulary.

**Hook-side gating（本变更合规修复）**: the claude driver's PreToolUse hook SHALL consult the session's currently effective mode (the shared mode unit maintained by spawn argv and runtime `SetMode`) before producing a permission request. Under a bypass tier (`allow`/`auto`) the hook SHALL resolve `allow` directly — no `PermissionRequest`, no permission card, silently and identically across every surface (Feishu, WebUI, and any other consumer of the same driver path). A runtime mode switch SHALL take effect for subsequent hook consults without respawning.

The Feishu permission card's `Allow session` button（「本会话不再询问」）SHALL act as a mode-switching surface: allow the current request and switch the session mode to `auto` (see「Three decision outcomes」).

Feishu-created sessions SHALL default to no mode (≈`ask`): every gated action asks. The mode, once set, lives on the session mapping (`desired_mode`) and SHALL be re-issued via the spawn argv on resume; `/new` and session end naturally return to the default tier.

#### Scenario: auto runs without prompting

- **WHEN** a session's mode is `auto` and its agent invokes a tool that would otherwise be gated
- **THEN** no permission request is produced and the tool proceeds

#### Scenario: desired and effective mode can differ

- **WHEN** the control plane sets a mode an execution body cannot enforce
- **THEN** the reported effective mode states what is actually enforced, and the difference is visible to the operator

#### Scenario: auto is an explicit choice

- **WHEN** a session is created without an explicit mode
- **THEN** it does not default to `auto`

#### Scenario: local claude session honors allow at creation

- **WHEN** a local claude session is created with mode `allow` and its agent invokes a gated tool
- **THEN** the tool proceeds without a permission request (bypassPermissions applied at spawn)

#### Scenario: local claude mid-session switch to edit relaxes edit gating

- **WHEN** a running local claude session is switched from `ask` to `edit` and then invokes a file-edit tool
- **THEN** the edit proceeds without a permission request while other gated categories still ask

#### Scenario: unknown mode is rejected, not degraded

- **WHEN** a create or switch request carries a mode outside the vocabulary
- **THEN** the request is rejected with an explicit error and the session's mode is unchanged

#### Scenario: hook side gate is silent across surfaces

- **WHEN** a claude session's effective mode is a bypass tier (`allow`/`auto`) and its PreToolUse hook fires for a gated tool
- **THEN** the hook resolves `allow` directly — no permission request and no card appears on any surface (Feishu or WebUI alike)

#### Scenario: runtime switch takes effect on the next hook consult

- **WHEN** a running claude session is switched from `ask` to `allow` mid-turn
- **THEN** the next gated tool invocation's hook consult sees the new mode and resolves `allow` without producing a request, no respawn required

#### Scenario: feishu created session defaults to asking

- **WHEN** a session is created from Feishu with no mode set and its agent invokes a gated tool
- **THEN** the permission card flow runs as usual (default ≈ `ask`)

#### Scenario: resume re-issues the session mode

- **WHEN** a session with `desired_mode` = `auto` is resumed
- **THEN** the mode is re-applied via the spawn argv and the hook gate stays silent from the first tool call

### Requirement: Remote approval requests survive control-plane absence

Permission requests raised by remote sessions SHALL travel to the control plane over the link and SHALL remain parked while the control plane is absent. On its return, the control plane SHALL present every request that is still parked, and SHALL NOT present one that was already resolved. A decision arriving for a session that has already terminated SHALL be discarded under the existing stale-click semantics.

#### Scenario: parked requests are presented after the control plane returns

- **WHEN** the control plane returns after an absence during which requests were parked on a node
- **THEN** every request still outstanding is presented for a decision

#### Scenario: a decision after session termination resolves nothing

- **WHEN** a decision arrives for a request whose session terminated while the control plane was away
- **THEN** the decision is discarded and the operator is told the request is no longer valid
