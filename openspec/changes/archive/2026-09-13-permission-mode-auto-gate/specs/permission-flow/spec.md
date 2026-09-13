## REMOVED Requirements

### Requirement: Auto-approve on allowlist hit

**Reason**: 「本会话不再询问」类自动放行统一收敛到会话 mode 门控（见「Session mode gates whether a decision is requested」的 hook 侧门控）：allowlist 是与 mode 语义重叠的第二套"别问我"机制，两套并存导致自动放行路径分叉、审计口径不一。driver 层 mode 成为唯一门控机制后，按签名白名单不再有独立存在价值（proposal Non-goals 明确不做按签名白名单）。

**Migration**: 存量 allowlist 条目无迁移价值——会话按其当前 mode 继续门控；需要"本会话别再问"的操作者改走权限卡「本会话不再询问」按钮（放行当前请求并切 `auto`）或 webui 的 mode 切换。

### Requirement: Allowlist scope and lifetime

**Reason**: allowlist 机制整体退役（见上），其生命周期规则随之失去载体。会话级"不再询问"的生命周期由 mode 承载：mode 存会话映射，`/new` 与会话结束自然回到默认档（不设 mode ≈ `ask`），无需独立清理逻辑。

**Migration**: 无。原「会话结束清空白名单」的语义由 mode 生命周期自然覆盖（新会话无 mode = 默认 ask，事事照问）。

## MODIFIED Requirements

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
