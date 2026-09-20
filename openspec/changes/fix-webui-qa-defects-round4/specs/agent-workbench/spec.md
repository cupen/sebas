## MODIFIED Requirements

### Requirement: Submit control reflects submission and turn state

The composer's submit control SHALL reflect the state of the input and the focused session's turn:

- empty input and no in-flight work: the control SHALL render disabled;
- non-empty input and no in-flight work: the control SHALL render as the enabled send affordance;
- a submission POST in flight: the control SHALL render an in-progress indication instead of the send affordance;
- the focused session has a turn in flight and the input is empty: the control SHALL render as a stop affordance, and activating it SHALL request cancellation of the session's in-flight turn;
- the focused session has a turn in flight and the input is non-empty: the control SHALL render a queued affordance, and submitting SHALL enqueue the submission via the existing turn queue without dropping or replacing pending entries;
- the focused session's child is starting (spawn requested or in flight, no live turn yet) with the input non-empty: the control SHALL render a starting affordance that is visually distinct from the queued affordance — the submission is staged for the starting child, and the workbench SHALL NOT present it as queued behind a running turn;
- when the turn ends (or the cancel completes), the control SHALL return to the send affordance.

"Turn in flight" SHALL mean the core-side truth that a turn occupies the session — a working phase OR a spawn window OR a turn parked on a pending permission request OR the accepted-receipt phase (the operator's submission has been accepted and recorded as the newest transcript entry while no agent output entry has landed yet) — and SHALL NOT be derived from a display-only status slug that renames those states (e.g. waiting). While the turn is in flight only because the session awaits the operator's permission decision, the control SHALL additionally indicate that the session is waiting on the operator, so submitting reads as queueing behind an answerable prompt rather than disappearing into an unexplained queue.

#### Scenario: empty input is disabled

- **WHEN** the input is empty and the focused session has no turn in flight
- **THEN** the submit control is disabled

#### Scenario: typed input enables send

- **WHEN** the operator types into the composer and the focused session has no turn in flight
- **THEN** the submit control becomes the enabled send affordance

#### Scenario: submission in flight shows progress

- **WHEN** a submission POST is in flight
- **THEN** the control shows an in-progress indication and returns to the send affordance when the request settles

#### Scenario: streaming with empty input offers stop

- **WHEN** the focused session is streaming a turn and the input is empty
- **THEN** the control renders as a stop affordance; activating it sends the cancel request and the control returns to the send affordance once the turn is no longer in flight

#### Scenario: accepted receipt without agent output offers stop

- **WHEN** the focused session's newest transcript unit is the operator's just-accepted submission, no agent output entry has landed, and the input is empty
- **THEN** the control renders as a stop affordance; activating it requests cancellation of the accepted turn instead of leaving the operator without a self-service exit

#### Scenario: streaming with text offers queueing

- **WHEN** the focused session is streaming a turn and the input is non-empty
- **THEN** the control renders a queued affordance and submitting appends the text to the pending stack

#### Scenario: starting child stages rather than queues

- **WHEN** the focused session's child is starting and the operator submits a message
- **THEN** the control renders the starting affordance, distinct from the queued affordance, and the message is staged for the child that is starting

#### Scenario: cancel does not drop pending submissions

- **WHEN** the operator cancels the in-flight turn while pending submissions are queued
- **THEN** the in-flight turn is interrupted and the pending submissions remain queued

#### Scenario: submission while a permission prompt is parked queues visibly

- **WHEN** the focused session's turn is parked on a pending permission request and the operator submits a message
- **THEN** the control shows the queued affordance together with an indication that the session waits on the operator's decision, and the submission lands in the pending stack instead of silently displacing the perceived send

#### Scenario: stop stays reachable while parked

- **WHEN** the focused session's turn is parked on a permission request and the input is empty
- **THEN** the control renders as a stop affordance so the operator can cancel the parked turn without hunting for the approval card
