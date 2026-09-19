## ADDED Requirements

### Requirement: 待批请求可按会话枚举（读模型）

The system SHALL expose the currently parked permission requests of a session through a read model: for each parked request the session key, the permission `request_id`, the tool name, and the tool arguments SHALL be retrievable while the request is undecided. The read model SHALL reflect additions and resolutions immediately, and SHALL be empty once no request is parked. This read model is independent of the push channel: event delivery (WS or Feishu card) remains the realtime surface, but the read model SHALL NOT depend on having received the original event.

#### Scenario: 未决请求可从读模型取得

- **WHEN** an agent turn parks a permission request (hook callback unanswered)
- **THEN** the session's read model lists that request with its `request_id`, tool name, and tool arguments

#### Scenario: 刷新或重连后仍可取得

- **WHEN** a client (re)opens the session or the page is reloaded while a request is parked
- **THEN** the read model still lists the pending request without any prior event delivery being required

#### Scenario: 批复后从读模型消失

- **WHEN** a parked request receives its decision
- **THEN** the read model no longer lists that request

### Requirement: Cancel 释放泊车审批（fail-closed）

When a turn is cancelled (stop-reply / interrupt) or a session is terminated while permission requests are parked, the system SHALL release every parked request of that turn fail-closed: no parked request survives as an orphan, and `turn_engaged` SHALL fall back to false once the turn has ended (no phase in flight AND no parked requests). A released request SHALL NOT be resolvable afterwards.

#### Scenario: 停止回复清空未决审批

- **WHEN** the operator stops the reply while a permission request is parked
- **THEN** every parked request of the session is released and the session reports no pending approvals

#### Scenario: 释放后回合状态复位

- **WHEN** a cancelled turn had parked requests that are now released
- **THEN** the session's engaged-turn state is false (the stop control no longer applies) and survives page reloads

#### Scenario: 释放的请求不可再批复

- **WHEN** a parked request has been released by a cancel
- **THEN** a later decision submission for that `request_id` is rejected instead of unblocking anything
