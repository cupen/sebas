# feishu-bridge Specification

## Purpose

Owns the Feishu WebSocket ingress and egress channel: long-connection lifecycle with reconnect backoff, inbound event parsing with deduplication and chat-type/mention gating, thread-aware reply targeting, and the outbound Feishu API calls (send card, update card, reactions) with token refresh and transient-error retry. In this change the bridge becomes the Feishu **adapter** implementing the neutral channel abstraction, no longer a core-domain type.

## MODIFIED Requirements

### Requirement: Feishu adapter implements the neutral channel abstraction

The Feishu WebSocket ingress and egress SHALL be exposed as an adapter implementing the core's neutral `channels` abstraction: it SHALL translate inbound Feishu wire events (text / media / button callback / form callback) into neutral inbound events addressed by `ChannelKey`, and SHALL translate the neutral outbound presentation model into Feishu card schema 2.0 API calls. The core SHALL interact with the adapter only through the neutral abstraction; the adapter SHALL own all Feishu-specific translation (session key shape, message ids, thread targets, card JSON, reactions).

#### Scenario: inbound feishu event becomes a neutral event

- **WHEN** a Feishu text message arrives with chat id `oc_x` and thread id `t1`
- **THEN** the adapter emits a neutral text event addressed by `ChannelKey("feishu", "oc_x", Some("t1"))`
- **AND** the concrete Feishu reply target (root message id) is carried in the event's channel-specific metadata, not surfaced to the core domain

#### Scenario: outbound neutral presentation renders as a feishu card

- **WHEN** the core emits a neutral outbound presentation for a session with channel key `feishu:oc_x`
- **THEN** the adapter renders it as a Feishu card (per `feishu-cards` rendering rules) and sends it via the Feishu API with thread-aware reply targeting

#### Scenario: feishu session is addressable by core via neutral key only

- **WHEN** the router holds a session whose originating channel is `feishu`
- **THEN** the router addresses it by the `ChannelKey` and never constructs Feishu chat/thread ids itself

### Requirement: Inbound event gating and execution routing

The feishu adapter SHALL gate inbound feishu events on: explicit feishu enablement (adapter registered); event deduplication; chat-type filtering against `allowed_chat_types`; group/p2p mention gating when `bot_name` is configured. After the gates pass, the adapter SHALL present the event to the core router as a neutral event; the router SHALL route it to a session execution body: by default the ACP bridge; when the session or an explicit configuration selects the native kernel, to the native `sebas-agent` session under the shared router state.

#### Scenario: Feishu disabled rejects inbound

- **WHEN** the feishu enable switch is off
- **THEN** no feishu adapter is registered, no WebSocket connection is established, and no inbound event is processed

#### Scenario: Feishu group mention gate still applies

- **WHEN** a group text message does not mention the bot while `bot_name` is configured
- **THEN** the message is dropped by the adapter and never reaches the router

#### Scenario: Native-routed feishu session does not render feishu cards

- **WHEN** a feishu message creates (or continues) a native-kernel session
- **THEN** no feishu card / reaction / text reply is emitted for that session's output
- **AND** the session's transcript is readable through the WebUI turn-content API

#### Scenario: ACP-routed feishu session renders cards as today

- **WHEN** a feishu message creates (or continues) an ACP-bridge session
- **THEN** the existing card / reaction / thread-reply behavior is unchanged