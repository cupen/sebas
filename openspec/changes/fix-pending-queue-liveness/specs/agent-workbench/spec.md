## MODIFIED Requirements

### Requirement: Submit control reflects submission and turn state

The composer's submit control SHALL reflect the state of the input and the focused session's turn:

- empty input and no in-flight work: the control SHALL render disabled;
- non-empty input and no in-flight work: the control SHALL render as the enabled send affordance;
- a submission POST in flight: the control SHALL render an in-progress indication instead of the send affordance;
- the focused session has a turn in flight and the input is empty: the control SHALL render as a stop affordance, and activating it SHALL request cancellation of the session's in-flight turn;
- the focused session has a turn in flight and the input is non-empty: the control SHALL render a queued affordance, and submitting SHALL enqueue the submission via the existing turn queue without dropping or replacing pending entries;
- when the turn ends (or the cancel completes), the control SHALL return to the send affordance.

"Turn in flight" SHALL mean the core-side truth that a turn occupies the session — a working phase OR a spawn window OR a turn parked on a pending permission request — and SHALL NOT be derived from a display-only status slug that renames those states (e.g. waiting). While the turn is in flight only because the session awaits the operator's permission decision, the control SHALL additionally indicate that the session is waiting on the operator, so submitting reads as queueing behind an answerable prompt rather than disappearing into an unexplained queue.

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

#### Scenario: streaming with text offers queueing

- **WHEN** the focused session is streaming a turn and the input is non-empty
- **THEN** the control renders a queued affordance and submitting appends the text to the pending stack

#### Scenario: cancel does not drop pending submissions

- **WHEN** the operator cancels the in-flight turn while pending submissions are queued
- **THEN** the in-flight turn is interrupted and the pending submissions remain queued

#### Scenario: submission while a permission prompt is parked queues visibly

- **WHEN** the focused session's turn is parked on a pending permission request and the operator submits a message
- **THEN** the control shows the queued affordance together with an indication that the session waits on the operator's decision, and the submission lands in the pending stack instead of silently displacing the perceived send

#### Scenario: submission during the spawn window queues visibly

- **WHEN** the focused session is in its spawn window (starting) and the operator submits a message
- **THEN** the control renders the queued affordance and the submission is presented as staged for the first message

#### Scenario: stop stays reachable while parked

- **WHEN** the focused session's turn is parked on a permission request and the input is empty
- **THEN** the control renders as a stop affordance so the operator can cancel the parked turn without hunting for the approval card

### Requirement: Pending submissions stack above the composer

The workbench SHALL render the focused session's pending submissions directly above the composer input, in delivery order, without occupying transcript space. Each entry SHALL state its disposition: submissions staged during a spawn SHALL read as combining into the session's first message, and submissions queued behind a running turn SHALL read as waiting, with their position. When the queue is not advancing, the stack SHALL state why — naming the blocking condition (the running turn, or the permission decision the operator owes) and when the wait began — so an idle-looking session with a growing stack is never unexplained. A submission SHALL NOT appear in the transcript until it starts (or is combined at activation); at that moment it SHALL leave the stack and the transcript SHALL show it as a submission entry.

The stack SHALL support removal of any entry and drag-reordering within the entry's own disposition group. Entries carrying priority (`/btw`) SHALL be rendered as priority and SHALL NOT be draggable, and no drag SHALL place a non-priority entry ahead of a priority one. Submitting while entries are pending SHALL append to the stack — it SHALL NOT replace, discard, or silently merge into an existing entry's text.

Rejections of removal or reorder SHALL be handled in two tiers: when the server's post-operation truth has already converged with the operator's intent (a race lost to a concurrent start), the stack SHALL silently reconcile; any other rejection (unknown entry, out-of-range target, priority conflict, or the entry still present after the operation) SHALL surface as a low-severity transient notice naming the entry and the reason — the stack SHALL NEVER fail silently on an operation the server did not perform.

When the session ends or is closed while entries are pending, those entries SHALL be reported as not executed in a single explicit notice naming them (the stack itself disappears with the session, so the notice is the record); the stack SHALL never shrink without an explanation.

#### Scenario: stack renders pending submissions in order

- **WHEN** the focused session has submissions staged during spawn and submissions queued behind a running turn
- **THEN** the region above the composer lists them in delivery order, distinguishing "combining into the first message" from "waiting" with position

#### Scenario: submitting appends instead of replacing

- **WHEN** the operator submits a message while entries are already pending
- **THEN** the new submission appears as an additional stack entry and no existing entry's text is altered

#### Scenario: remove and reorder take effect

- **WHEN** the operator removes a pending entry, or drags a non-priority entry to a new position in its group
- **THEN** the stack reflects the change immediately and still reflects it after the next refresh

#### Scenario: priority entries are pinned

- **WHEN** the stack contains a `/btw` priority entry
- **THEN** it is rendered as priority, cannot be dragged, and no drag can place another entry ahead of it

#### Scenario: a started submission leaves the stack and enters the transcript

- **WHEN** a pending submission starts its turn (or is combined at activation)
- **THEN** it is removed from the stack and appears in the transcript as a submission entry, so the conversation shows what is actually running

#### Scenario: pending entries are not silently dropped at session end

- **WHEN** the focused session fails or is closed while entries are pending
- **THEN** a notice names those entries as not executed, and the stack clears only together with that notice

#### Scenario: the stack explains a queue that is not advancing

- **WHEN** entries have been pending because the running turn is parked on the operator's permission decision
- **THEN** the stack names that condition and when the wait began, instead of rendering bare position labels on a session that looks idle

#### Scenario: deterministic rejection is visible

- **WHEN** a remove or reorder is rejected with a typed error (unknown entry, out-of-range, priority conflict) and the server's pending list still contains the entry
- **THEN** a transient low-severity notice names the entry and the reason, and the stack reconciles to the server's list

#### Scenario: race lost to a concurrent start reconciles silently

- **WHEN** a remove or reorder targets an entry that started its turn between the render and the click, so the server's post-operation list no longer contains it
- **THEN** the stack silently reconciles to the server's list without a notice
