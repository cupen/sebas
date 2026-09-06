# core-session-channel Specification（delta）

## MODIFIED Requirements

### Requirement: Session drive methods

The channel SHALL provide methods to create a session with a prompt and an
optional project directory, send a message to an existing session, close a
session, and cancel a session's in-flight turn. Each SHALL return either the
resulting `ChannelKey` or a typed rejection naming the reason. A create request
carrying a project directory SHALL have that path canonicalized and verified to
be an existing directory before any child is spawned, and SHALL be rejected
otherwise. Spawned sessions SHALL be registered under the requesting client's
channel (e.g. `web` for the WebUI).

The channel SHALL additionally provide IM-frontend semantics for chat-shaped
clients: a message request MAY carry ensure semantics under which the core
creates a session for an unknown key (lazily respawning a dormant session)
instead of rejecting it, preserving each channel's historical auto-spawn
behavior without requiring the client to hold mapping state.

A message request (plain or ensure) MAY carry attachments — locally resolvable
file references (path + mime type + file name), typically images — delivered
alongside the text. The core SHALL verify each attachment's path exists before
delivery; a request with a missing attachment SHALL be rejected with a typed
reason and no message delivered.

#### Scenario: create spawns a real session

- **WHEN** a client requests session creation with a prompt
- **THEN** the core spawns a real ACP session under the client's channel, returns
  its `ChannelKey`, and subscribers observe the new session

#### Scenario: unusable project directory rejected

- **WHEN** a create request names a path that is not an existing directory
- **THEN** the request is rejected with a reason and no child is spawned

#### Scenario: message to unknown session rejected

- **WHEN** a message or close request names a `ChannelKey` the core does not know
- **THEN** the response is a typed rejection and nothing is mutated

#### Scenario: IM 前端对未知 key 的文本自动建会话

- **WHEN** an IM frontend sends a text message with ensure semantics for a key
  the core does not know
- **THEN** the core creates the session (as an inbound text historically did) and
  delivers the message to it

#### Scenario: IM 前端对 dormant 会话的文本复活会话

- **WHEN** an IM frontend sends a text message with ensure semantics for a
  dormant session key
- **THEN** the core lazily respawns the session and delivers the message

#### Scenario: 消息携带图片附件投给执行体

- **WHEN** an IM frontend sends a message whose attachments include an
  existing local image file
- **THEN** the core delivers the text and the image to the session's
  execution body as model-visible content, and the request's response reports
  acceptance

#### Scenario: 附件路径不存在被拒绝

- **WHEN** a message request names an attachment path that does not exist
- **THEN** the response is a typed rejection and nothing is delivered to the
  session

#### Scenario: cancel terminates the in-flight turn

- **WHEN** a client requests cancellation for a session with a turn in flight
- **THEN** the core cancels that turn and the session stays usable for
  subsequent messages

## ADDED Requirements

### Requirement: Approval requests surface from every execution body

The subscription stream SHALL surface permission/approval requests from every
execution body — the native kernel's gated tool calls and the ACP bridge's
permission requests alike — as answerable frames carrying the request id, the
tool name, and the session's `ChannelKey`. A decision returned via the approval
answer method SHALL be routed by the core to the originating execution body by
request id alone. Requests with no reachable client SHALL fail closed at the
originating execution body. This extends the native-only approval surfacing
(wire-webui-sebas-agent-e2e) to ACP-bridge sessions so a detached IM frontend
can render the same permission cards it rendered when it shared the core's
process.

#### Scenario: ACP 权限请求出现在订阅流

- **WHEN** an ACP-bridge session raises a tool permission request
- **THEN** a connected IM frontend receives an approval frame naming the
  request id and session key, and can render a permission card from it

#### Scenario: 决定按 request_id 回路由

- **WHEN** the IM frontend answers an approval frame with a decision
- **THEN** the core routes the decision to the ACP session by request id and the
  parked tool call resumes or is denied accordingly

#### Scenario: 无客户端连接时 fail-closed

- **WHEN** an approval request arises while no frontend is connected to answer it
- **THEN** the request fails closed at the execution body, matching the native
  kernel's existing posture
