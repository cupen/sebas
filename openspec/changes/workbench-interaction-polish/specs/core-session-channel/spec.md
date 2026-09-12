# core-session-channel Delta

## ADDED Requirements

### Requirement: Session cancel over the channel

The channel SHALL expose a cancel method that forwards an operator's cancel for a session, addressed by the neutral session key, to the core. The core SHALL route it to that session's cancel mechanism — the driver's existing interrupt semantics — so the in-flight turn is interrupted while the child and session survive for subsequent turns. Pending submissions SHALL NOT be dropped by a cancel: entries queued behind the interrupted turn MAY start afterwards. Cancelling a session that has no turn in flight SHALL be answered with a typed rejection stating the session is idle rather than a fabricated success; cancelling an unknown session key SHALL be answered with a typed rejection. When the core is unreachable, the method SHALL fail with the channel's existing honest-degradation semantics.

#### Scenario: cancel interrupts the in-flight turn

- **WHEN** the cancel method is invoked for a session whose turn is in flight
- **THEN** the in-flight turn is interrupted via the session's cancel mechanism, the child survives, and the session is available for subsequent turns

#### Scenario: pending submissions survive a cancel

- **WHEN** a session with queued pending submissions has its in-flight turn cancelled
- **THEN** the pending submissions are not dropped and may start once the interrupted turn has ended

#### Scenario: cancel on an idle session is a typed rejection

- **WHEN** the cancel method is invoked for a session that has no turn in flight
- **THEN** the response is a typed rejection stating the session is idle, not a success

#### Scenario: cancel on an unknown session is a typed rejection

- **WHEN** the cancel method is invoked with a session key that does not resolve
- **THEN** the response is a typed rejection naming the unknown session
