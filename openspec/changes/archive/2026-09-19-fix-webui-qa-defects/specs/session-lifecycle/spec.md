## MODIFIED Requirements

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
