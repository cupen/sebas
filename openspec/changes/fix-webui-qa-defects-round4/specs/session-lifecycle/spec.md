## MODIFIED Requirements

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
