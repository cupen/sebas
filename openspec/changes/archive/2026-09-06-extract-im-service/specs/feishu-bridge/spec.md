# feishu-bridge Specification（delta）

## MODIFIED Requirements

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

## REMOVED Requirements

### Requirement: Inbound media events pass file keys only

**Reason**: This behavior is superseded by the service split. After the IM service extraction only the im process holds Feishu credentials, so a raw Feishu `file_key` crossing the core session channel is unresolvable by the core — the historical `[attached: <file_key>]` prompt marker would deliver no image content to the model, which is unacceptable for an agent workbench that must handle images.

**Migration**: The im service resolves inbound media itself (see `im-service`): it downloads the payload per the `[media]` configuration and presents the neutral media event with a locally resolvable reference (path + mime + file name). The core session channel carries attachments as first-class, optional message fields (see `core-session-channel`).

## ADDED Requirements

### Requirement: Inbound media resolves to a usable attachment

The feishu adapter SHALL parse inbound image/file/audio messages into media events that the im service resolves into locally usable attachments: the payload SHALL be downloaded to the configured media directory (size-capped, streamed to disk), and the event presented onward SHALL carry the local file reference (path + mime type + file name) instead of the raw Feishu `file_key`. The agent-visible prompt SHALL carry the image content itself (per the execution body's ingestion path), not only a textual marker.

#### Scenario: image message reaches the model as content

- **WHEN** an inbound image message passes the gates and media download succeeds
- **THEN** the session request carries the downloaded file's local path and mime type
- **AND** the execution body receives the image as model-visible content (native Image block; ACP image content block when the agent negotiates the capability)

#### Scenario: media download failure is honest

- **WHEN** the Feishu media download fails or the file exceeds the size cap
- **THEN** the user is told the attachment could not be delivered and why, and no session message claims success
