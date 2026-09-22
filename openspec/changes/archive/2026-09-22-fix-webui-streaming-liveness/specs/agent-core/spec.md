# agent-core Delta

## MODIFIED Requirements

### Requirement: Approval-first webui surface and event vocabulary

The system SHALL support the webui as the first approval answerer: it SHALL emit `PermissionRequest` events for policy-gated calls and SHALL define the decision vocabulary that drives the webui review card as the approval seam (permission decision requests) — the `PermissionRequest` event plus a stable policy-decision result outcome (emitted as the `ToolPolicy` event with outcome `allowed_once | allowed_session | escalated | denied | unavailable`). The kernel SHALL NOT render UI itself.

native 执行体的事件词汇 SHOULD 包含工具执行进度事件：长时间运行的工具（如 bash、web）宜在执行期间发出进度事件，供呈现面（webui、IM 卡）在工具开始与结束之间展示活跃状态；进度事件 MUST NOT 计入未读。本变更范围内该词汇已定义但接线为可选后续（proposal 标注「可选」，tasks 未列）——呈现面在事件缺席时按工具开始/结束边界呈现，不得因此视为缺陷。事件词汇的呈现面消费规则沿用既有审批与事件路由语义。

#### Scenario: The approval decision is a distinct event outcome

- **WHEN** a gated call is answered through the webui seam
- **THEN** the kernel ends the permission flow with a stable policy-decision outcome (the `ToolPolicy` event), distinct from a normal tool finish

#### Scenario: long tool call shows progress

- **WHEN** a native session runs a tool that takes more than a few seconds and the progress vocabulary is wired
- **THEN** progress events flow to the presentation surface between tool start and tool end, so the turn does not appear frozen

#### Scenario: progress events do not create unread

- **WHEN** progress events arrive while the operator is away
- **THEN** the session unread count is unchanged by them
