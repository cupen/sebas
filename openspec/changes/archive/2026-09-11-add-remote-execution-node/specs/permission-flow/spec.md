## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: Session mode gates whether a decision is requested

Each session SHALL carry a mode (`ask`, `edit`, `allow`, `auto`, and any further values the control plane defines) that decides whether a tool action needs a decision at all. Under `auto` the session SHALL run without producing permission requests. The mode SHALL be a desired value held by the control plane, and the execution side SHALL report the mode it actually enforces. An execution body that cannot enforce a mode SHALL report that fact rather than appearing to enforce it. `auto` SHALL NOT be the default mode and SHALL leave an audit trail when selected.

#### Scenario: auto runs without prompting

- **WHEN** a session's mode is `auto` and its agent invokes a tool that would otherwise be gated
- **THEN** no permission request is produced and the tool proceeds

#### Scenario: desired and effective mode can differ

- **WHEN** the control plane sets a mode an execution body cannot enforce
- **THEN** the reported effective mode states what is actually enforced, and the difference is visible to the operator

#### Scenario: auto is an explicit choice

- **WHEN** a session is created without an explicit mode
- **THEN** it does not default to `auto`

### Requirement: Remote approval requests survive control-plane absence

Permission requests raised by remote sessions SHALL travel to the control plane over the link and SHALL remain parked while the control plane is absent. On its return, the control plane SHALL present every request that is still parked, and SHALL NOT present one that was already resolved. A decision arriving for a session that has already terminated SHALL be discarded under the existing stale-click semantics.

#### Scenario: parked requests are presented after the control plane returns

- **WHEN** the control plane returns after an absence during which requests were parked on a node
- **THEN** every request still outstanding is presented for a decision

#### Scenario: a decision after session termination resolves nothing

- **WHEN** a decision arrives for a request whose session terminated while the control plane was away
- **THEN** the decision is discarded and the operator is told the request is no longer valid
