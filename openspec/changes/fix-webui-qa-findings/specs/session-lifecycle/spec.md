## MODIFIED Requirements

### Requirement: Terminal error teardown

On a terminal agent error, the system SHALL, in order: finalize the card with the failure, clear the chat's permission allowlist, clear the reply-target entry, remove the mapping (dropping any pending submissions), drop the card state, and clear the root message id. The next message in that chat SHALL start a fresh session. Dropped pending submissions SHALL be reported to the session's observers as not executed — the drop SHALL never be silent.

Teardown SHALL clear only the live binding and runtime state. The session's persisted record and transcript SHALL survive teardown: operators SHALL still find the session in the session list, its detail and transcript SHALL remain retrievable, and the terminal turn SHALL remain visible as a failure in that transcript. Removing the session record itself (list disappearance or detail unavailability) SHALL NOT be part of terminal teardown.

A child process that dies mid-turn (crash, non-zero exit, vanished without a result frame) SHALL be detected and finalized into a terminal state promptly: the turn SHALL NOT linger in a running/queued presentation for the full stall-watchdog window when the child's death is already observable. A message submitted after the child's death SHALL either spawn a fresh session immediately or be rejected with a typed, visible cause — it SHALL NOT be parked in a state whose only exit is the stall watchdog's forced sweep. The forced sweep MAY remain as a safety net, but it SHALL NOT be the primary path by which a crashed session becomes usable again.

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

#### Scenario: child crash is finalized without the stall watchdog

- **WHEN** the agent child process exits mid-turn without emitting a result (crash) and the driver reports the death
- **THEN** the turn reaches a terminal failure state promptly (bounded by the driver's own death detection, not the stall-watchdog window), with a visible error entry
- **AND** the session does not present a running/queued state for the full stall-watchdog duration

#### Scenario: message after a crash is not zombie-parked

- **WHEN** the operator sends a message to a session whose child has crashed and whose turn has been finalized as failed
- **THEN** the message either spawns a fresh session immediately or is rejected with a typed visible cause
- **AND** it is not left parked in a queued state whose only exit is the stall watchdog's forced sweep

## ADDED Requirements

### Requirement: 取消请求即时确认

When the operator issues a cancel (stop) for an in-flight turn, the UI SHALL
present an immediate acknowledgement of the pending cancel (composer control
state and/or a transcript notice) at click time — the acknowledgement SHALL
NOT wait for the agent's next output. If the backend cannot interrupt the
child immediately (silent period), the cancel SHALL remain visually pending
and the turn SHALL be finalized as cancelled as soon as the child becomes
responsive or exits; the system SHALL NOT present the turn as running with
no indication that a cancel is pending during that window. The final
cancelled outcome SHALL appear in the transcript once applied.

#### Scenario: 点击停止即时有反馈

- **WHEN** the operator clicks stop while the agent is in a silent period
- **THEN** within the click's response cycle the composer shows the cancel as
  pending (not a plain running state), before any agent output arrives

#### Scenario: 取消最终生效并留痕

- **WHEN** a pending cancel reaches the child and the turn is finalized
- **THEN** the transcript shows the turn was stopped, and the session is
  reusable for a follow-up message
