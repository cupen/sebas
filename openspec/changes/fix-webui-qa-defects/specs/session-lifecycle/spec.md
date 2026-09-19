## MODIFIED Requirements

### Requirement: Lazy spawn on first message

The system SHALL NOT pre-create sessions. The first text message in an unmapped chat (or thread) SHALL atomically create a Spawning placeholder and emit a spawn instruction; subsequent messages SHALL be routed once the session activates. A placeholder that exists without a first message (`awaiting_first_prompt`) SHALL NOT count as a turn in flight: it MUST NOT arm the turn-stall watchdog, MUST NOT be force-settled by it, and MUST NOT have synthetic error entries (such as turn-stall notices) appended to its transcript. The watchdog MAY only consider a session stalled once a real turn — triggered by a message or an explicit activation — has begun.

#### Scenario: First text spawns a session

- **WHEN** a text message arrives for a key with no mapping
- **THEN** a Spawning placeholder is inserted synchronously under the session map's single write lock
- **AND** a spawn instruction is emitted

#### Scenario: Mapping activation drains queued messages

- **WHEN** the spawned session completes its handshake
- **THEN** the mapping transitions to Active with the new session id
- **AND** messages staged during spawning are drained, in arrival order, as one combined prompt (joined with newlines)
- **AND** those staged submissions leave the observable pending stack at the moment they are combined

#### Scenario: idle placeholder is never stall-settled

- **WHEN** a placeholder session created without a prompt sits idle longer than the configured turn-stall timeout
- **THEN** no stall watchdog event fires for it, no synthetic error entry is appended to its transcript, and its status remains writable

#### Scenario: stall watchdog still guards real turns

- **WHEN** a real turn on an active session produces no events for longer than the configured turn-stall timeout
- **THEN** the watchdog force-settles that turn as today
