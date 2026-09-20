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

The channel SHALL additionally provide pending-submission management: remove a
pending submission by id, and reorder a pending submission to a new position
inside its own disposition group. Both SHALL be rejected with a typed reason
when the id is unknown, when the submission has already started, or when the
requested order would move a non-priority submission ahead of a priority one —
never silently ignored.

Every deployment shape that exposes the session surface SHALL reach this
management plane, including deployments where the web UI fronts a composite of
multiple execution backends: the composite SHALL route each management call to
the backend that hosts the target session's queue, and the hosting backend's
typed rejections SHALL pass through verbatim. A missing route on the composite
SHALL NOT surface as generic unavailability, as a core-unreachable claim, or as
any 5xx whose message misattributes the cause; until the route exists the
operation MUST fail loudly and honestly name the gap.

#### Scenario: create spawns a real session

- **WHEN** a client requests session creation with a prompt
- **THEN** the core spawns a real ACP session under the client's channel, returns
  its `ChannelKey`, and subscribers observe the new session

#### Scenario: unusable project directory rejected

- **WHEN** a create request names a path that is not an existing directory
- **THEN** the request is rejected with a reason and no child is spawned

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

#### Scenario: removing a pending submission over the channel

- **WHEN** a client removes a pending submission by id before it starts
- **THEN** the core removes it from the queue, and subsequent snapshots and
  events no longer list it

#### Scenario: managing an already-started submission is rejected

- **WHEN** a client removes or reorders a submission that has already started
- **THEN** the response is a typed rejection stating that it is already running,
  and the in-flight turn is unaffected

#### Scenario: reorder from the web UI in an embedded deployment

- **WHEN** the operator moves or removes one of several queued submissions from
  the web UI while the core is reachable (bare-core embedded shape, sessions on
  the acp bridge)
- **THEN** the queue reorders or drops the submission and every connected
  client observes the new delivery order without a page reload

#### Scenario: typed rejections pass through the composite

- **WHEN** a management call names an unknown id, an already-started
  submission, or an order that would break the priority prefix invariant
- **THEN** the caller receives the hosting backend's typed rejection reason
  (unknown / already started / priority conflict / out of range), not a generic
  unavailable or unreachable error
