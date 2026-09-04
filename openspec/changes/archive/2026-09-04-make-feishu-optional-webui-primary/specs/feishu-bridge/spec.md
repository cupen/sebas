## ADDED Requirements

### Requirement: Inbound event gating and execution routing

The system SHALL gate inbound feishu events on: explicit feishu enablement (when disabled, no inbound processing or outbound calls occur); event deduplication; chat-type filtering against `allowed_chat_types`; group/p2p mention gating when `bot_name` is configured. After the gates pass, the system SHALL route the event to a session execution body: by default the ACP bridge; when the session or an explicit configuration selects the native kernel, to the native `sebas-agent` session under the shared router state.

#### Scenario: Feishu disabled rejects inbound

- **WHEN** the feishu enable switch is off
- **THEN** no WebSocket connection is established and no inbound event is processed

#### Scenario: Feishu group mention gate still applies

- **WHEN** a group text message does not mention the bot while `bot_name` is configured
- **THEN** the message is dropped regardless of execution body

#### Scenario: Native-routed feishu session does not render feishu cards

- **WHEN** a feishu message creates (or continues) a native-kernel session
- **THEN** no feishu card / reaction / text reply is sent for that session's output
- **AND** the session's transcript is readable through the WebUI turn-content API

#### Scenario: ACP-routed feishu session renders cards as today

- **WHEN** a feishu message creates (or continues) an ACP-bridge session
- **THEN** the existing card / reaction / thread-reply behavior is unchanged