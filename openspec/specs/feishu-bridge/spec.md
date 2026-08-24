# feishu-bridge Specification

## Purpose
Owns the Feishu WebSocket ingress and egress channel: long-connection lifecycle with reconnect backoff, inbound event parsing with deduplication and chat-type/mention gating, thread-aware reply targeting, and the outbound Feishu API calls (send card, update card, reactions) with token refresh and transient-error retry.

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

### Requirement: Inbound media events pass file keys only

The system SHALL parse inbound image/file/audio messages into file keys and compose them into the agent prompt as attachment markers (e.g. `[attached: <file_key>]`). The media payload itself SHALL NOT be downloaded in this path.

#### Scenario: Image message becomes attachment marker

- **WHEN** an inbound image message arrives
- **THEN** the router receives a media event carrying the message's file key
- **AND** the agent prompt contains an attachment marker rather than downloaded file content
