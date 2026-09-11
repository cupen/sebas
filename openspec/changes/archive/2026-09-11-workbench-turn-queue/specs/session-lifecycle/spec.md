## MODIFIED Requirements

### Requirement: Lazy spawn on first message

The system SHALL NOT pre-create sessions. The first text message in an unmapped chat (or thread) SHALL atomically create a Spawning placeholder and emit a spawn instruction; subsequent messages SHALL be routed once the session activates.

#### Scenario: First text spawns a session

- **WHEN** a text message arrives for a key with no mapping
- **THEN** a Spawning placeholder is inserted synchronously under the session map's single write lock
- **AND** a spawn instruction is emitted

#### Scenario: Mapping activation drains queued messages

- **WHEN** the spawned session completes its handshake
- **THEN** the mapping transitions to Active with the new session id
- **AND** messages staged during spawning are drained, in arrival order, as one combined prompt (joined with newlines)
- **AND** those staged submissions leave the observable pending stack at the moment they are combined

### Requirement: Double-spawn race protection

While a spawn is in flight for a key, the system SHALL stage incoming messages instead of spawning again. The stage SHALL be capped at 16 submissions. A submission that would exceed the cap SHALL be rejected visibly at the submission surface (a typed rejection carrying the reason), SHALL NOT be silently discarded, and SHALL NOT displace staged submissions. Staging and draining SHALL be race-free against a concurrent spawn activation. A second `/new` during an in-flight spawn SHALL be ignored.

#### Scenario: Second text during spawn is queued

- **WHEN** a second text message arrives while the session for that key is still Spawning
- **THEN** the message is appended to the pending queue and no second spawn is emitted

#### Scenario: Queue overflow drops the newest

- **WHEN** a message arrives while the pending queue already holds 16 messages
- **THEN** the incoming message is dropped and the submitting client is told so with a typed rejection naming the cap as the reason
- **AND** no already-staged submission is displaced, and no code path reports the submission as accepted

#### Scenario: Rapid duplicate /new spawns once

- **WHEN** `/new` is issued twice in quick succession for the same key
- **THEN** only one spawn instruction is emitted and the second command is ignored

### Requirement: Terminal error teardown

On a terminal agent error, the system SHALL, in order: finalize the card with the failure, clear the chat's permission allowlist, clear the reply-target entry, remove the mapping (dropping any pending submissions), drop the card state, and clear the root message id. The next message in that chat SHALL start a fresh session. Dropped pending submissions SHALL be reported to the session's observers as not executed — the drop SHALL never be silent.

#### Scenario: Terminal error cleans up all chat state

- **WHEN** a session dies with a terminal error
- **THEN** the card receives a failure state, allowlist and reply-target for the key are cleared, and the mapping is removed
- **AND** pending submissions awaiting the dead session are dropped, never delivered to a later session
- **AND** each dropped submission is reported as not executed to observers that were watching the session

#### Scenario: Next message after death spawns fresh

- **WHEN** a text message arrives after the mapping was torn down by a terminal error
- **THEN** the lazy-spawn path runs as if the chat had never had a session

### Requirement: Turn queue back-pressure while streaming

While a session is actively streaming a turn, an incoming text message SHALL be enqueued (not sent) with a waiting acknowledgement, rather than interleaved. This SHALL hold for every channel that can submit text, including the `web` channel driven by the WebUI — no channel SHALL bypass back-pressure by writing the prompt into the transcript at submission time. The queued submission SHALL enter the session transcript only when it actually starts its turn. Priority messages (`/btw`) SHALL be inserted at the front of the queue.

#### Scenario: Message during active turn queues

- **WHEN** a text message arrives while the session's current turn is still in flight
- **THEN** the message is enqueued and acknowledged with a waiting indicator
- **AND** it is delivered only after the current turn completes

#### Scenario: Web submission does not interleave with the streaming turn

- **WHEN** the WebUI submits a message while the session is streaming a turn
- **THEN** the submission is queued and appears in the observable pending stack, not in the transcript
- **AND** the transcript receives a submission entry for it only when its turn starts, so the running turn's output is never interleaved by a not-yet-started submission

#### Scenario: /btw jumps the queue

- **WHEN** `/btw <text>` arrives while earlier turns sit in the queue
- **THEN** the priority message is placed ahead of them and is delivered first

## ADDED Requirements

### Requirement: Pending submissions are observable and manageable

Every submission the core has accepted but not yet started SHALL be observable to clients as a pending submission carrying a stable id, its text, its position, and its disposition:

- `staging` — accepted before the session existed (spawn window); it will be combined with its siblings into one prompt at activation;
- `turn` — accepted while a turn was in flight; it will run as its own turn, in order.

A priority (`/btw`) submission SHALL be observable as such and SHALL remain ahead of non-priority submissions. Clients SHALL be able to remove a pending submission by id and to reorder it within its own disposition group. Reordering SHALL NOT move a submission across disposition groups and SHALL NOT move a non-priority submission ahead of a priority one. Once a submission has left the pending stack (its turn started, or it was combined at activation), remove and reorder requests against its id SHALL be rejected with a typed reason rather than silently ignored.

#### Scenario: pending submission exposes id, order and disposition

- **WHEN** a client observes a session with submissions staged during spawn and submissions queued behind a streaming turn
- **THEN** every not-yet-started submission is listed with a stable id, its text, its position and its disposition (`staging` or `turn`)

#### Scenario: removing a pending submission

- **WHEN** a client removes a pending submission by id before it starts
- **THEN** it no longer appears in the pending stack and is never delivered to the agent

#### Scenario: reordering within a disposition group

- **WHEN** a client reorders a non-priority pending submission to a new position inside its own group
- **THEN** subsequent delivery follows the new order

#### Scenario: priority submissions stay ahead

- **WHEN** a client attempts to reorder a non-priority submission ahead of a priority one
- **THEN** the request is rejected with a typed reason and the order is unchanged

#### Scenario: a started submission can no longer be managed

- **WHEN** a client removes or reorders a submission whose turn has already started
- **THEN** the request is rejected with a typed reason stating that it is already running, and the running turn is unaffected
