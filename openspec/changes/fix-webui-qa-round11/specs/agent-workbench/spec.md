## ADDED Requirements

### Requirement: Message bubbles preserve submitted line breaks

Multi-line submissions SHALL be rendered with their line breaks preserved in
the conversation: user bubbles and agent entries derived from them SHALL
render each submitted line on its own line (pre-wrap semantics). The wire
format already carries the newlines; the transcript renderer MUST NOT collapse
them into spaces. Long single-token lines keep the existing overflow handling.

#### Scenario: three-line message renders as three lines

- **WHEN** the operator submits a message containing three lines (two `\n`)
- **THEN** the user bubble renders the text as three visual lines
- **AND** an agent entry echoing the same text also renders three lines

#### Scenario: reload keeps the line structure

- **WHEN** the page is reloaded after a multi-line exchange
- **THEN** the restored transcript still renders the submitted line breaks

### Requirement: Closed-session deep link settles without polling

When the focused session cannot be loaded because it does not exist or has
been closed, the workbench SHALL present the existing clear unavailable state
and SHALL stop re-requesting that session's data: at most one failing fetch
per navigation, no retry loop. Any background refresh that re-enters the
unavailable session (e.g. a periodic sync) SHALL skip the unavailable session
instead of repeating the failing request. The presentation (centered notice,
error entry, disabled composer) SHALL NOT change.

#### Scenario: deep link to a closed session stops after one miss

- **WHEN** the operator opens the deep link of a closed session and stays on the page
- **THEN** the unavailable state is presented after a single failing load
- **AND** the console records at most one 404 for that session instead of a continuous stream

#### Scenario: reload of an unavailable deep link behaves the same

- **WHEN** the page is reloaded while focused on the unavailable session
- **THEN** the same single-miss behavior applies, with no polling loop
