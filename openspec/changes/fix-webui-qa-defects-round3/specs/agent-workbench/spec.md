## ADDED Requirements

### Requirement: Creation confirmation activates immediately

The session creation dialog SHALL activate its confirmation control on the first
activation attempt — pointer click or keyboard activation alike — regardless of
whether the operator interacted with the agent picker or any other control inside
the dialog beforehand. While a creation request is in flight, the dialog SHALL
present a busy state on the confirmation control and SHALL ignore further
activation attempts instead of issuing duplicate creation requests.

#### Scenario: First click after picking an agent creates the session

- **WHEN** the operator opens the creation dialog, selects a non-default agent from
  the agent picker, and clicks the confirmation control once
- **THEN** exactly one session creation request is issued and the dialog resolves
  (closes on success, or stays open with an inline cause on failure)
- **AND** the confirmation control does not require a second activation

#### Scenario: In-flight creation shows busy state and ignores re-activation

- **WHEN** a creation request has been issued and the operator clicks the
  confirmation control again before it resolves
- **THEN** no additional creation request is issued
- **AND** the control presents a busy (in-flight) state for the duration of the
  request
