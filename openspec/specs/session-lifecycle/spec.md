# session-lifecycle Specification

## Purpose
Owns the mapping between channel conversations and agent sessions: per-thread session identity, lazy spawn on first message, race-safe spawn bookkeeping, dormant sessions restored across daemon restarts, turn queuing with back-pressure, and the full cleanup contract when a session dies.

## Requirements

### Requirement: Session identity is per chat and thread

The system SHALL key session mappings by the channel-neutral session identity
(`ChannelKey`: channel name plus channel-specific opaque reference, see
`channels` and `openspec/glossary.md`). For the feishu channel the opaque
reference continues to distinguish chat and topic, so a chat holding multiple
topics still holds multiple independent mappings. Web UI sessions continue to
use synthetic references with no thread component, now under the `web` channel.

#### Scenario: Two topics in one chat are separate sessions

- **WHEN** messages arrive from two different topics in the same chat
- **THEN** each topic maps to its own session with its own conversation history

#### Scenario: Main chat maps independently of topics

- **WHEN** a message arrives in the chat's main thread while topics exist in the same chat
- **THEN** the main thread maps to its own session, separate from any topic session

### Requirement: Lazy spawn on first message

The system SHALL NOT pre-create sessions. The first text message in an unmapped chat (or thread) SHALL atomically create a Spawning placeholder and emit a spawn instruction; subsequent messages SHALL be routed once the session activates. Spawn instruction emission and child activation for one session SHALL be independent of every other session: another session's live turn, in-flight spawn, or queued submissions SHALL NOT delay or suppress this session's spawn instruction or its activation. The webui workbench SHALL be able to hold several sessions with live children at the same time, bounded only by the existing capacity limit. A placeholder that exists without a first message (`awaiting_first_prompt`) SHALL NOT count as a turn in flight: it MUST NOT arm the turn-stall watchdog, MUST NOT be force-settled by it, and MUST NOT have synthetic error entries (such as turn-stall notices) appended to its transcript. The watchdog MAY only consider a session stalled once a real turn — triggered by a message or an explicit activation — has begun.

#### Scenario: First text spawns a session

- **WHEN** a text message arrives for a key with no mapping
- **THEN** a Spawning placeholder is inserted synchronously under the session map's single write lock
- **AND** a spawn instruction is emitted

#### Scenario: Mapping activation drains queued messages

- **WHEN** the spawned session completes its handshake
- **THEN** the mapping transitions to Active with the new session id
- **AND** messages staged during spawning are drained, in arrival order, as one combined prompt (joined with newlines)
- **AND** those staged submissions leave the observable pending stack at the moment they are combined

#### Scenario: Second session spawns while the first is working

- **WHEN** a text message is submitted to a second (0-turn placeholder or dormant) session while the first session's child is streaming a turn
- **THEN** the second session's spawn instruction is emitted and its child starts without waiting for the first session's turn to end
- **AND** both sessions can hold live children and make progress concurrently

#### Scenario: A failed spawn does not strand submissions

- **WHEN** a session's spawn fails (for example the agent command cannot start)
- **THEN** the mapping enters the typed failed state with the reason recorded, the staged submissions are no longer held as queueable, and the failure is observable on the session row/detail
- **AND** the next message submitted to that session retries the spawn instead of being appended to a queue that nothing will ever drain

#### Scenario: idle placeholder is never stall-settled

- **WHEN** a placeholder session created without a prompt sits idle longer than the configured turn-stall timeout
- **THEN** no stall watchdog event fires for it, no synthetic error entry is appended to its transcript, and its status remains writable

#### Scenario: stall watchdog still guards real turns

- **WHEN** a real turn on an active session produces no events for longer than the configured turn-stall timeout
- **THEN** the watchdog force-settles that turn as today

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

### Requirement: Dormant sessions resume lazily

Sessions persisted across a daemon restart SHALL be restored as Dormant. A Dormant mapping reads as dead for liveness purposes. The first text on a Dormant mapping SHALL atomically claim a resume (replacing the mapping with a Spawning placeholder so concurrent messages queue rather than double-resume) and emit a resume instruction for the old session id. `/new` on a Dormant mapping SHALL mean a fresh spawn, not a resume.

#### Scenario: First text after restart resumes

- **WHEN** a text message arrives for a key whose mapping is Dormant
- **THEN** the mapping is claimed for resume under the write lock and a resume instruction referencing the old session id is emitted

#### Scenario: Dormant /new starts fresh

- **WHEN** `/new` is issued for a key whose mapping is Dormant
- **THEN** a fresh session is spawned (replacing the dormant mapping) rather than resuming

#### Scenario: Rejected resume falls back to fresh

- **WHEN** the agent rejects the resume of an old conversation id
- **THEN** a fresh session is started under a new id
- **AND** the user is informed that the old conversation is gone

### Requirement: Terminal error teardown

On a terminal agent error, the system SHALL, in order: finalize the card with the failure, clear the chat's permission allowlist, clear the reply-target entry, remove the mapping (dropping any pending submissions), drop the card state, and clear the root message id. The next message in that chat SHALL start a fresh session. Dropped pending submissions SHALL be reported to the session's observers as not executed — the drop SHALL never be silent.

Teardown SHALL clear only the live binding and runtime state. The session's persisted record and transcript SHALL survive teardown: operators SHALL still find the session in the session list, its detail and transcript SHALL remain retrievable, and the terminal turn SHALL remain visible as a failure in that transcript. Removing the session record itself (list disappearance or detail unavailability) SHALL NOT be part of terminal teardown.

#### Scenario: Terminal error cleans up all chat state

- **WHEN** a session dies with a terminal error
- **THEN** the card receives a failure state, allowlist and reply-target for the key are cleared, and the mapping is removed
- **AND** pending submissions awaiting the dead session are dropped, never delivered to a later session
- **AND** each dropped submission is reported as not executed to observers that were watching the session

#### Scenario: Next message after death spawns fresh

- **WHEN** a text message arrives after the mapping was torn down by a terminal error
- **THEN** the lazy-spawn path runs as if the chat had never had a session

#### Scenario: Escalation kill keeps the session browsable

- **WHEN** the driver's hang kill ladder terminates the child during a turn and the driver emits its terminal error
- **THEN** the affected turn is finalized with a visible error-class transcript entry naming the cause
- **AND** the session remains present in the session list and its transcript remains retrievable via the session APIs
- **AND** no notification-free removal of the session record occurs

### Requirement: Expected turn completion keeps the session

A completed turn (agent Finished) SHALL NOT remove the mapping. Queued turns, if any, SHALL be drained to the still-live session for the next round.

#### Scenario: Queued turn runs after completion

- **WHEN** a turn completes while another turn is waiting in the queue
- **THEN** the queued turn is sent to the same session as the next prompt

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

### Requirement: Capacity limit on concurrent sessions

The system SHALL enforce a configured maximum number of concurrent sessions. Spawn attempts beyond the limit SHALL be rejected with a capacity error and SHALL NOT displace existing mappings.

#### Scenario: Spawn beyond capacity is rejected

- **WHEN** a spawn would exceed the configured session capacity
- **THEN** the spawn is rejected with a capacity error

### Requirement: Restart recovery with corruption tolerance

On daemon start, the system SHALL restore the persisted session map from the state store: an empty store yields an empty table; restored entries become Dormant. Session-map entries SHALL be written per mutation, so that an unclean exit preserves every committed mapping and shutdown ordering no longer decides what survives. The daemon SHALL never refuse to start because of session-map state itself: a session map whose entries cannot be read is reported honestly and the daemon starts with an empty table. A state store that cannot be opened at all is governed by the state store's corruption rule and SHALL NOT be reset or recreated by session-map recovery.

#### Scenario: Corrupt session map is quarantined

- **WHEN** the persisted session-map entries cannot be read at startup
- **THEN** the unreadable data is set aside rather than presented as valid, with the reason logged
- **AND** the daemon starts with an empty session table
- **AND** a store that cannot be opened at all follows the state store's corruption rule instead (refuse to start with a diagnostic, never reset)

#### Scenario: Missing file starts empty

- **WHEN** no persisted session map exists at startup
- **THEN** the daemon starts with an empty table and no error

#### Scenario: Snapshot precedes shutdown kill

- **WHEN** the daemon shuts down while sessions are active
- **THEN** every mapping committed before shutdown is already durable in the state store
- **AND** shutdown ordering cannot lose a mapping, because no shutdown-time snapshot is required to persist it

#### Scenario: Unclean exit keeps the mapping

- **WHEN** the daemon is killed without a graceful shutdown while sessions are active
- **THEN** after restart the session map contains every mapping committed before the kill
- **AND** it does not depend on a snapshot having been written at the last graceful shutdown

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

### Requirement: Remote session lifespan is bound to its node

A session placed on an execution node SHALL keep running while the control plane is absent: a dropped link SHALL NOT terminate it, and a control-plane restart SHALL NOT terminate it. The session's life SHALL instead be bound to the node that runs it — a node restart terminates the sessions it was running, and the control plane SHALL report them as terminated rather than as still running. Reconciliation SHALL NOT create a replacement session for one that is still alive on its node.

#### Scenario: control-plane restart does not end remote sessions

- **WHEN** the control plane restarts while sessions are active on a node
- **THEN** those sessions keep running on the node
- **AND** after reconnection the control plane reports them as the same sessions, with their history continued rather than restarted

#### Scenario: link loss does not end remote sessions

- **WHEN** the websocket between the control plane and a node drops and is re-established
- **THEN** no session on that node is terminated or replaced

#### Scenario: node restart ends its sessions

- **WHEN** a node process restarts
- **THEN** the sessions it was running are reported as terminated with the node named as the cause
- **AND** the control plane does not present them as live or as silently resumable

#### Scenario: terminated is not recreated

- **WHEN** the control plane reconciles with a node that no longer holds a session it previously knew
- **THEN** that session is marked terminated and no new session is created in its place

### Requirement: Spawn independence is observable in the workbench

While one session is working, the workbench SHALL present other sessions' startup progress from their own lifecycle state: a session whose child is starting SHALL be presented as starting (with its submissions staged), never as queued behind another session's turn, and a session whose spawn failed SHALL present the recorded reason at the surface where the operator submitted. The rail's per-session status SHALL reflect each session's own state without cross-session coupling.

#### Scenario: submitting to a placeholder while another session streams

- **WHEN** the operator sends the first message to a 0-turn placeholder while a different session is streaming
- **THEN** the placeholder's row moves to starting and the composer's submission is shown as staged for startup, not as a queued turn

#### Scenario: failed spawn names the cause

- **WHEN** a session's spawn fails and the operator focuses or submits to it
- **THEN** the workbench states the failure reason inline at the composer and the submission surface allows retry

### Requirement: Session map is persisted per mutation

Each committed change to a session's mapping SHALL be durable in the state store before it is observable to clients, so that an abrupt process exit cannot roll a mapping back to an earlier state. The session map SHALL NOT be persisted only at shutdown.

#### Scenario: A committed mapping survives a kill

- **WHEN** a session is created and its mapping is observable to a client, and the daemon is then killed immediately
- **THEN** after restart the mapping is present with the same session identity and desired mode

#### Scenario: Rolling back a mapping is not observable

- **WHEN** a mapping change is committed and a client then reads the session map
- **THEN** the read reflects the committed change
- **AND** a subsequent abrupt exit does not revert it

### Requirement: Session state vocabulary is shared and closed

The session state vocabulary — the session phase reported by the control plane and by an execution node, and the session mode (`desired` / `effective`) — SHALL be carried by a single shared definition rather than by ad-hoc strings redeclared per layer. The set of accepted values SHALL be closed and their spellings SHALL be fixed: the control plane and the execution node SHALL agree on one value set even though each emits only the subset valid for its own role. An unrecognized value received from a peer SHALL NOT fail deserialization, drop the message, or close the connection; it SHALL be carried as an explicitly-marked unknown value. The user-facing status shown by each channel surface SHALL be derived from the phase by a type-checked mapping, never by matching on string literals.

#### Scenario: Both sides share one value set with fixed spellings

- **WHEN** the control plane reports a session phase and an execution node reports the same session's phase
- **THEN** both values come from the same shared definition
- **AND** every value retains the exact spelling it had before this vocabulary was typed (including hyphenated forms such as the spawn-failure phase)

#### Scenario: An unknown phase from a newer peer is tolerated

- **WHEN** a peer sends a phase value this build does not know
- **THEN** the message is still accepted and the session remains observable
- **AND** the value is surfaced as an unknown phase rather than causing a deserialization failure or a dropped frame

#### Scenario: Adding a phase value cannot silently miss a handler

- **WHEN** a new phase value is added to the shared definition
- **THEN** every consumer that branches on the phase fails to compile until it handles the new value
- **AND** no consumer relies on a string comparison that would silently take a default branch instead

#### Scenario: Display status is derived, not string-matched

- **WHEN** a channel surface renders a session's status for the user
- **THEN** the status is produced by mapping the shared phase value to the presentation status
- **AND** a phase whose presentation mapping is missing is a compile-time omission, not a silently wrong label

#### Scenario: Session mode crosses every layer as one type

- **WHEN** a session's desired or effective mode is read, set, or forwarded through the channel, the node link, or the web UI request surface
- **THEN** it is carried as the shared mode type rather than as a free-form string
- **AND** an unrecognized mode from a peer is tolerated as an unknown mode instead of being rejected
