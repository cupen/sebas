## ADDED Requirements

### Requirement: Pending queue advances without depending on a single terminal event

The core SHALL NOT allow a session's pending submission queue to stall indefinitely behind a turn that never settles. When a session's turn has been in flight (working phase), no permission decision is parked on it, and no event of any kind has arrived for that session for a configurable continuous duration (`turn_stall_timeout`, default 600 seconds, 0 disables the guard), the core SHALL force-settle the turn to a terminal phase, drain the queue head, and SHALL emit a visible warning notice naming the session and the number of stalled submissions it released. A turn parked on a pending permission request SHALL be exempt from the stall guard for as long as it stays parked. The guard SHALL NOT fire while the session is streaming events, regardless of turn duration.

#### Scenario: stalled turn is force-settled and the queue drains

- **WHEN** a session's turn has been working with zero events for longer than `turn_stall_timeout` and no permission is parked
- **THEN** the core settles the turn to a terminal phase, starts the next queued submission (if any), and a warning notice names the session and the stalled submissions released

#### Scenario: parked permission does not trip the stall guard

- **WHEN** a session's turn has been waiting on a permission request longer than `turn_stall_timeout`
- **THEN** the guard does not fire; the turn stays parked until the operator answers or cancels it

#### Scenario: active streaming never trips the stall guard

- **WHEN** a session has been streaming a long-running turn with events arriving continuously
- **THEN** the guard does not fire regardless of total turn duration

#### Scenario: stall guard can be disabled

- **WHEN** `turn_stall_timeout` is configured as `0`
- **THEN** no stall detection runs and behavior matches the pre-guard semantics
