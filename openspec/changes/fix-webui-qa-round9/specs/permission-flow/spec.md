## ADDED Requirements

### Requirement: Approval answer surfacing and observability

The review card SHALL bound its `answering` state: if the answer POST neither succeeds nor fails within 10 seconds, the card SHALL return to a retryable pending state with a visible error naming the timeout, instead of remaining silently disabled. The core SHALL log every approval answer it accepts (request_id, decision, session) at info level, so a click that never took effect can be distinguished from one that was never sent by comparing the browser network log with the core log.

#### Scenario: answer request hangs

- **WHEN** the operator clicks a decision and the POST to the answer endpoint never completes
- **THEN** within 10 seconds the card shows a visible timeout error, leaves the buttons enabled for retry, and the turn remains parked (fail-closed unchanged)

#### Scenario: answer accepted is traceable

- **WHEN** an answer POST reaches the core and is delivered to the parked request
- **THEN** the core log contains one info line naming the request_id, the decision, and the session, correlating with the browser's network log
