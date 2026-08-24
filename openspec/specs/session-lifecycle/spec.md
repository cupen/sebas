# session-lifecycle Specification

## Purpose
Owns the mapping between Feishu conversations and agent sessions: per-thread session identity, lazy spawn on first message, race-safe spawn bookkeeping, dormant sessions restored across daemon restarts, turn queuing with back-pressure, and the full cleanup contract when a session dies.

## Requirements

### Requirement: Session identity is per chat and thread

The system SHALL key session mappings by the pair (chat id, thread id). A chat holding multiple topics therefore holds multiple independent mappings. Web UI sessions SHALL use synthetic unique keys with no thread component.

#### Scenario: Two topics in one chat are separate sessions

- **WHEN** messages arrive from two different topics in the same chat
- **THEN** each topic maps to its own session with its own conversation history

#### Scenario: Main chat maps independently of topics

- **WHEN** a message arrives in the chat's main thread while topics exist in the same chat
- **THEN** the main thread maps to its own session, separate from any topic session

### Requirement: Lazy spawn on first message

The system SHALL NOT pre-create sessions. The first text message in an unmapped chat (or thread) SHALL atomically create a Spawning placeholder and emit a spawn instruction; subsequent messages SHALL be routed once the session activates.

#### Scenario: First text spawns a session

- **WHEN** a text message arrives for a key with no mapping
- **THEN** a Spawning placeholder is inserted synchronously under the session map's single write lock
- **AND** a spawn instruction is emitted

#### Scenario: Mapping activation drains queued messages

- **WHEN** the spawned session completes its handshake
- **THEN** the mapping transitions to Active with the new session id
- **AND** messages queued during spawning are drained, in arrival order, as one combined prompt (joined with newlines)

### Requirement: Double-spawn race protection

While a spawn is in flight for a key, the system SHALL queue incoming messages instead of spawning again. The queue SHALL be capped at 16 messages; overflow drops the newest message with a warning. A second `/new` during an in-flight spawn SHALL be ignored.

#### Scenario: Second text during spawn is queued

- **WHEN** a second text message arrives while the session for that key is still Spawning
- **THEN** the message is appended to the pending queue and no second spawn is emitted

#### Scenario: Queue overflow drops the newest

- **WHEN** a message arrives while the pending queue already holds 16 messages
- **THEN** the incoming message is dropped with a warning log

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

On a terminal agent error, the system SHALL, in order: finalize the card with the failure, clear the chat's permission allowlist, clear the reply-target entry, remove the mapping (dropping any queued turns), drop the card state, and clear the root message id. The next message in that chat SHALL start a fresh session.

#### Scenario: Terminal error cleans up all chat state

- **WHEN** a session dies with a terminal error
- **THEN** the card receives a failure state, allowlist and reply-target for the key are cleared, and the mapping is removed
- **AND** queued turns awaiting the dead session are dropped, never delivered to a later session

#### Scenario: Next message after death spawns fresh

- **WHEN** a text message arrives after the mapping was torn down by a terminal error
- **THEN** the lazy-spawn path runs as if the chat had never had a session

### Requirement: Expected turn completion keeps the session

A completed turn (agent Finished) SHALL NOT remove the mapping. Queued turns, if any, SHALL be drained to the still-live session for the next round.

#### Scenario: Queued turn runs after completion

- **WHEN** a turn completes while another turn is waiting in the queue
- **THEN** the queued turn is sent to the same session as the next prompt

### Requirement: Turn queue back-pressure while streaming

While a session is actively streaming a turn, an incoming text message SHALL be enqueued (not sent) with a waiting acknowledgement, rather than interleaved. Priority messages (`/btw`) SHALL be inserted at the front of the queue.

#### Scenario: Message during active turn queues

- **WHEN** a text message arrives while the session's current turn is still in flight
- **THEN** the message is enqueued and acknowledged with a waiting indicator
- **AND** it is delivered only after the current turn completes

#### Scenario: /btw jumps the queue

- **WHEN** `/btw <text>` arrives while earlier turns sit in the queue
- **THEN** the priority message is placed ahead of them and is delivered first

### Requirement: Capacity limit on concurrent sessions

The system SHALL enforce a configured maximum number of concurrent sessions. Spawn attempts beyond the limit SHALL be rejected with a capacity error and SHALL NOT displace existing mappings.

#### Scenario: Spawn beyond capacity is rejected

- **WHEN** a spawn would exceed the configured session capacity
- **THEN** the spawn is rejected with a capacity error

### Requirement: Restart recovery with corruption tolerance

On daemon start, the system SHALL restore the persisted session map: a missing or empty file yields an empty table; a valid file restores all entries as Dormant; a corrupt file is quarantined (renamed aside) and the daemon starts with an empty table — the daemon SHALL never refuse to start over session-map state. On daemon shutdown, the snapshot SHALL be written before sessions are killed.

#### Scenario: Corrupt session map is quarantined

- **WHEN** the persisted session map file contains invalid JSON at startup
- **THEN** the file is renamed to a quarantine name
- **AND** the daemon starts with an empty session table

#### Scenario: Missing file starts empty

- **WHEN** the session map file does not exist at startup
- **THEN** the daemon starts with an empty table and no error

#### Scenario: Snapshot precedes shutdown kill

- **WHEN** the daemon shuts down
- **THEN** the session map snapshot is written to disk before any session process is killed
