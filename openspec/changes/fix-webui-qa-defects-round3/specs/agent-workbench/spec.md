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

### Requirement: Focused session arrivals never badge

While a session is the focused session and the document is visible, its rail
row SHALL NOT present an unread badge, whatever the stored read anchor
currently says — the unread badge exists for arrivals in unfocused sessions,
and a focused session's watched arrivals become read through the transcript
advancing the shared read anchor (bottom-follow). The suppression SHALL NOT
apply while the document is hidden: arrivals observed in a background tab
SHALL present the badge as usual. Re-activating the focused session's row (a
same-session no-op switch) SHALL re-advance the read anchor to the row's
current message count, so a badge that appeared while the session was
unfocused is cleared by the click that focuses it and stays cleared on
repeated clicks.

#### Scenario: Streaming arrival into the focused session does not badge

- **WHEN** visible reply segments arrive — streamed or via a snapshot
  refetch — in the focused session while the document is visible
- **THEN** the session's rail row presents no unread badge

#### Scenario: Repeated focus keeps the badge cleared

- **WHEN** an unfocused session shows an unread badge and the operator clicks
  its row, then clicks the same row again without switching away
- **THEN** the first click clears the badge by advancing the stored read
  anchor to the row's current message count
- **AND** the repeated same-session click leaves the badge cleared
