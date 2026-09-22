## MODIFIED Requirements

### Requirement: Hang detection with escalating kill

The system SHALL detect a hung child while a turn is active and run a kill ladder: `interrupt()` up to 3 times, then disconnect (≈SIGTERM), then drop (≈SIGKILL). This "kill ladder" escalation is distinct from the approval `escalate` decision (a native-kernel one-shot allow). Hang detection SHALL be suspended while a permission request is parked awaiting user click, and SHALL NOT fire when no turn is active. The hang kill ladder is implemented for the Claude driver; a generic ACP child that hangs mid-turn is not yet escalated against. When the ladder terminates the child, the affected turn SHALL be finalized as a visible failure and the session SHALL remain observable: the session record and transcript SHALL be preserved (the operator can see what happened and what was produced before the hang), and pending submissions SHALL be released per the pending-queue semantics with explicit reporting.

#### Scenario: No activity during a turn triggers escalation

- **WHEN** the child produces no message for the configured hang timeout while a turn is active
- **THEN** the driver issues `interrupt()` up to 3 times
- **AND** if the child still does not respond, disconnects the transport
- **AND** if the child persists, drops the client (SIGKILL)

#### Scenario: Escalation finalizes the turn without erasing the session

- **WHEN** the kill ladder terminates a hung child mid-turn
- **THEN** the affected turn settles with a visible error-class entry carrying the escalation cause
- **AND** the session remains listed with its full transcript retrievable
- **AND** queued submissions are released with explicit not-executed reporting instead of being silently discarded with the session record

#### Scenario: Permission wait is never a hang

- **WHEN** a PreToolUse permission request is parked awaiting user click
- **THEN** the hang detector is suspended
- **AND** the child is not interrupted regardless of elapsed time

#### Scenario: Idle child is not killed

- **WHEN** no turn is active (the child is waiting for the next prompt)
- **THEN** hang detection does not fire
