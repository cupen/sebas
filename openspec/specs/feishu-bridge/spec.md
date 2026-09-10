# feishu-bridge Specification

## Purpose
Owns the Feishu WebSocket ingress and egress channel: long-connection lifecycle with reconnect backoff, inbound event parsing with deduplication and chat-type/mention gating, thread-aware reply targeting, and the outbound Feishu API calls (send card, update card, reactions) with token refresh and transient-error retry.

The bridge is implemented as the Feishu channel adapter behind the core's neutral channel abstraction (`channels`): it owns every Feishu-specific translation and registers into the adapter registry only when feishu is enabled (decouple-feishu-channel).

## Requirements

### Requirement: WebSocket long connection with backoff reconnect

The system SHALL maintain a WebSocket long connection to Feishu and reconnect automatically on clean disconnects. Reconnect backoff SHALL start at 1 second and double per consecutive failure, capped at 60 seconds; a successful handshake resets backoff to 1 second.

#### Scenario: Server closes the connection

- **WHEN** the WebSocket closes cleanly or the server ends the stream
- **THEN** the system logs the closure and reconnects after 1 second

#### Scenario: Transient connect failure backs off exponentially

- **WHEN** consecutive connection attempts fail with non-auth errors
- **THEN** each retry waits twice as long as the previous one, up to a maximum of 60 seconds

### Requirement: Fatal connection errors stop the loop

The system SHALL permanently stop the WebSocket loop — without reconnecting — on authentication failure at runtime or on dispatcher registration failure at connect time. Startup token fetch failure SHALL abort process startup.

#### Scenario: Runtime auth failure exits the loop

- **WHEN** the WebSocket returns an authentication error during operation
- **THEN** the loop exits without any reconnect attempt

#### Scenario: Startup token fetch failure aborts startup

- **WHEN** the initial Feishu tenant token cannot be fetched at daemon start
- **THEN** the daemon fails to start with a Feishu error

### Requirement: Inbound event deduplication

The system SHALL drop inbound events whose `event_id` has already been seen. The seen-set SHALL be memory-bounded: when it exceeds 4096 ids it is cleared wholesale (accepting a small duplicate window). Events carrying no `event_id` bypass dedup. The seen-set is per-connection and is reset on reconnect.

#### Scenario: Duplicate event_id is dropped

- **WHEN** an event arrives whose `event_id` is already in the seen-set
- **THEN** the event is dropped with a debug log and never reaches the router

#### Scenario: Events without event_id always pass

- **WHEN** an event carries no `event_id` in its header
- **THEN** dedup does not apply and the event is processed

### Requirement: Chat type filtering

The system SHALL only process inbound events whose chat type is in the configured `allowed_chat_types` list. The default list SHALL be `private` and `group`. An empty configured list SHALL allow all chat types. Events from disallowed chat types SHALL be dropped silently.

#### Scenario: Disallowed chat type is dropped

- **WHEN** an event arrives with a chat type not in the allowed list
- **THEN** the event is dropped with a debug log and no user-facing signal

#### Scenario: Empty allowlist admits everything

- **WHEN** `allowed_chat_types` is configured as an empty list
- **THEN** events of any chat type are processed

### Requirement: Bot mention gating in chat and p2p

When `bot_name` is configured, the system SHALL process a text message only if one of its mentions contains the bot name (case-insensitive substring match on mention name or key). This gate SHALL apply to both `group` and `p2p` chat types, and SHALL NOT apply when `bot_name` is empty or when the chat type is neither group nor p2p. Media events and card/button callbacks carry no mentions and SHALL never be dropped by this gate. The mention text SHALL NOT be stripped from the message body.

#### Scenario: Unmentioned group message is dropped

- **WHEN** a group text message arrives mentioning the bot nowhere while `bot_name` is configured
- **THEN** the message is dropped

#### Scenario: Mention by key matches

- **WHEN** a text message's mention key (e.g. `@_user_1`) resolves to the configured bot name
- **THEN** the message is processed and the raw text (including the mention token) is forwarded as-is

#### Scenario: Empty bot_name disables the gate

- **WHEN** no `bot_name` is configured
- **THEN** all text messages pass the mention gate regardless of mentions

### Requirement: Thread-aware reply targeting

The system SHALL compute a reply target for each inbound message: for a message inside a topic thread, the root message id of the thread; for the topic's root message itself, its own message id; for a main-thread chat message, the triggering message id. Outbound cards in topics SHALL carry the thread id (query parameter) and the root id (body field) so replies land inside the thread.

#### Scenario: Topic child reply goes to thread root

- **WHEN** a message arrives from inside a topic thread with both `thread_id` and `root_id`
- **THEN** the session key includes the thread id and the reply target is the root message id

#### Scenario: Topic root replies to itself

- **WHEN** the topic's own root message arrives (has `thread_id`, no `root_id`)
- **THEN** the reply target is that message's own id

#### Scenario: Main-chat reply targets the triggering message

- **WHEN** a message arrives in the main thread of a chat
- **THEN** the reply target is that message's id and no thread routing applies

### Requirement: Invalid-topic errors force session close

When an outbound card send fails with Feishu error codes 230019 or 230071 (invalid topic), the system SHALL NOT retry the send; it SHALL deliver an error notice as plain text and SHALL force-close the affected session (idempotently).

#### Scenario: Topic invalid error handling

- **WHEN** sending a card into a topic fails with code 230019 or 230071
- **THEN** no retry is attempted for that card
- **AND** an error notice is sent as a text message
- **AND** the session for that chat is closed

### Requirement: Outbound API retry with token refresh

The system SHALL expose four outbound operations: send card, send text, update card, and add/remove reaction. When an outbound call returns a business error (non-zero code), the system SHALL force a tenant-token refresh and retry, for at most 3 total attempts per call. Transport-level failures SHALL surface immediately without retry.

#### Scenario: Business error retries with fresh token

- **WHEN** an outbound call returns a non-zero business code (e.g. expired token)
- **THEN** the token is force-refreshed and the call retried
- **AND** after 3 total failed attempts the error is surfaced

#### Scenario: Transport error is not retried

- **WHEN** an outbound HTTP call fails at the transport level
- **THEN** the error surfaces immediately without retry

### Requirement: Inbound event gating and execution routing

The feishu adapter SHALL gate inbound feishu events on: explicit feishu enablement (adapter registered); event deduplication; chat-type filtering against `allowed_chat_types`; group/p2p mention gating when `bot_name` is configured. All gates and the WebSocket loop SHALL run inside the IM service process (`im-service`). After the gates pass, the adapter SHALL present the event to the IM frontend as a neutral event; the frontend SHALL route it through the core session channel to a session execution body: by default the ACP bridge; when the session or an explicit configuration selects the native kernel, to the native `sebas-agent` session under the shared router state.

#### Scenario: Feishu disabled rejects inbound

- **WHEN** the feishu enable switch is off
- **THEN** no feishu adapter is registered, no WebSocket connection is established, and no inbound event is processed

#### Scenario: Feishu group mention gate still applies

- **WHEN** a group text message does not mention the bot while `bot_name` is configured
- **THEN** the message is dropped by the adapter and never reaches the IM frontend

#### Scenario: Native-routed feishu session does not render feishu cards

- **WHEN** a feishu message creates (or continues) a native-kernel session
- **THEN** no feishu card / reaction / text reply is emitted for that session's output
- **AND** the session's transcript is readable through the WebUI turn-content API

#### Scenario: ACP-routed feishu session renders cards as today

- **WHEN** a feishu message creates (or continues) an ACP-bridge session
- **THEN** the existing card / reaction / thread-reply behavior is unchanged

#### Scenario: core 不在 im 进程内

- **WHEN** im 进程处理一条入站飞书文本
- **THEN** 会话的创建、复活与投递经核心会话通道完成；im 进程内不存在会话映射或 agent 子进程

### Requirement: Feishu adapter implements the neutral channel abstraction

The Feishu WebSocket ingress and egress SHALL be exposed as an adapter implementing the core's neutral `channels` abstraction, hosted by the IM service process: it SHALL translate inbound Feishu wire events (text / media / button callback / form callback) into neutral inbound events addressed by `ChannelKey`, and SHALL translate the neutral outbound presentation model into Feishu card schema 2.0 API calls. The IM frontend SHALL interact with the adapter only through the neutral abstraction; the adapter SHALL own all Feishu-specific translation (session key shape, message ids, thread targets, card JSON, reactions). The core process SHALL NOT link or host the adapter.

#### Scenario: inbound feishu event becomes a neutral event

- **WHEN** a Feishu text message arrives with chat id `oc_x` and thread id `t1`
- **THEN** the adapter emits a neutral text event addressed by `ChannelKey("feishu", "oc_x", Some("t1"))`
- **AND** the concrete Feishu reply target (root message id) is carried in the event's channel-specific metadata, not surfaced to the core domain

#### Scenario: outbound neutral presentation renders as a feishu card

- **WHEN** the IM frontend emits a neutral outbound presentation for a session with channel key `feishu:oc_x`
- **THEN** the adapter renders it as a Feishu card (per `feishu-cards` rendering rules) and sends it via the Feishu API with thread-aware reply targeting

#### Scenario: feishu session is addressable by core via neutral key only

- **WHEN** the core holds a session whose originating channel is `feishu`
- **THEN** the core addresses it by the `ChannelKey` and never constructs Feishu chat/thread ids itself

### Requirement: Inbound media resolves to a usable attachment

The feishu adapter SHALL parse inbound image/file/audio messages into media events that the im service resolves into locally usable attachments: the payload SHALL be downloaded to the configured media directory (size-capped) and the event presented onward carries the local file reference instead of the raw Feishu `file_key`. **Deferred**：把图片升级为 model-visible content（native Image block / ACP image content block）与 mime 识别、wire `file_key` 解析目前未实现——当前投递形态是「`[图片已接收: …]` 文本标记 + 本地路径附件」，完整媒体内容链路见 beads `sebas-03s`（P1 媒体链路）。

#### Scenario: image message reaches the session as a path attachment

- **WHEN** an inbound image message passes the gates and media download succeeds
- **THEN** the session request carries the downloaded file's local path as a text-delivered attachment marker
- **AND** upgrade to model-visible image content is a tracked deferral (sebas-03s), not yet implemented

#### Scenario: media download failure is honest

- **WHEN** the Feishu media download fails or the file exceeds the size cap
- **THEN** the user is told the attachment could not be delivered and why, and no session message claims success
