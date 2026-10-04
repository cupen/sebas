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
and SHALL NOT loop: each fetcher that probes the unavailable session (detail,
activate, parked approvals, periodic sync) SHALL fire at most once per
navigation, and background refreshes that re-enter the unavailable session
SHALL skip it instead of repeating the failing request. The presentation
(centered notice, error entry, disabled composer) SHALL NOT change.

#### Scenario: deep link to a closed session stops after one miss

- **WHEN** the operator opens the deep link of a closed session and stays on the page
- **THEN** the unavailable state is presented after the initial failing loads
- **AND** no fetcher retries the unavailable session in a loop (request count per fetcher is bounded at one per navigation, with no periodic re-entry)

#### Scenario: reload of an unavailable deep link behaves the same

- **WHEN** the page is reloaded while focused on the unavailable session
- **THEN** the same bounded single-miss behavior applies, with no polling loop
