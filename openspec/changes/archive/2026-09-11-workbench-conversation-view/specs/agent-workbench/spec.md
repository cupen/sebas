## MODIFIED Requirements

### Requirement: Unseen-turn seam

The turn stream SHALL mark the boundary between turns the operator has already
seen and those that arrived since, showing how many arrived and over what span.
When a session has unseen turns, opening it SHALL position the stream at that
boundary rather than at the newest turn. The seen-boundary SHALL be per-browser
state and SHALL NOT be recorded server-side. The boundary SHALL fall between two turns and SHALL count turns rather than transcript entries, so one agent turn rendered as a single bubble is never split across the seam.

#### Scenario: opening a session with unseen turns

- **WHEN** the operator opens a session that received turns since their last
  visit
- **THEN** the stream opens positioned at the boundary, with the boundary
  marked and the count of turns below it stated

#### Scenario: nothing unseen

- **WHEN** the operator opens a session with no turns since their last visit
- **THEN** no boundary is drawn and the stream opens at the newest turn

#### Scenario: boundary is per-browser

- **WHEN** the operator opens the same session from a different browser
- **THEN** that browser's own seen-boundary applies, and the server holds no
  record of either

#### Scenario: the seam never splits a turn

- **WHEN** the operator opens a session whose unseen turns include one long agent turn composed of streamed text, thinking and tool calls
- **THEN** the boundary is drawn above that whole turn and no part of it appears on the seen side
- **AND** the stated count is the number of turns below the boundary, not the number of transcript entries

### Requirement: Model selector offers the backend catalog before any session

The composer's model selector SHALL offer the catalog the operator configured in
Settings — every configured provider's model list, presented as two levels
(provider, then model) — before any session exists, so the operator can pick a
model for the first turn of a new session. The default selection SHALL follow
the configured default provider and model. When a session exists, the selector SHALL offer that session's
`available_models` instead, because a mid-session switch is valid only if the
session's execution body accepts the chosen model.
When neither a configured catalog nor a session model list is available, the
selector SHALL state its unavailability honestly rather than offering an empty
or fabricated list. Changes to the configured catalog or the default SHALL be
reflected without requiring a session to be created first. The selector SHALL
NOT derive its options from another session's `available_models`.

#### Scenario: selector populated before any session

- **WHEN** a default provider with a models catalog is configured and the
  operator opens a fresh workbench with no sessions
- **THEN** the model selector offers the catalog's models

#### Scenario: provider and model are chosen in two levels

- **WHEN** the operator opens a fresh workbench with two providers configured in Settings
- **THEN** the selector first offers the providers, and choosing one offers that provider's models

#### Scenario: existing sessions keep session-sourced options

- **WHEN** a session exposes `available_models` and the operator opens its
  model dropdown
- **THEN** the selector offers that session's options, matching the current
  behavior

#### Scenario: no catalog and no session models is stated honestly

- **WHEN** no default provider catalog exists and no session exposes models
- **THEN** the selector presents an explicit unavailability indication rather
  than an empty list

## ADDED Requirements

### Requirement: Workbench renders the focused session as a conversation

The workbench SHALL render the focused session as a conversation between the
operator and the agent, in transcript order: each submission the operator made
SHALL appear as their own turn, and each agent turn SHALL appear as a single
assistant bubble. One agent turn SHALL be composed of everything the agent
produced for that turn — the streamed text concatenated in arrival order, its
thinking, and its tool invocations — and SHALL NOT be rendered as a series of
per-chunk bubbles. Thinking SHALL be folded inside the turn's bubble, and tool
invocations SHALL be grouped inside the bubble as an expandable "used N tools"
group rather than presented as ordinary prose. A submission SHALL appear in the
conversation only when its turn starts.

#### Scenario: both sides of the conversation are visible

- **WHEN** the operator opens a session in which they submitted messages across several turns
- **THEN** the workbench shows their submissions and the agent's replies in transcript order, each submission as the operator's own turn

#### Scenario: one agent turn is one bubble

- **WHEN** an agent turn arrives as many streamed text chunks plus thinking plus tool invocations
- **THEN** the workbench renders one assistant bubble for that turn, with the text in order, thinking folded inside, and the tool invocations collected in one expandable group

#### Scenario: tool invocations are not mistaken for prose

- **WHEN** an agent turn invokes tools
- **THEN** those invocations are rendered as the turn's tool group and are distinguishable from the turn's prose

#### Scenario: a submission appears when its turn starts

- **WHEN** a submission is accepted while the agent is still working
- **THEN** it is not rendered as a started turn until its turn actually begins

### Requirement: Workbench is the single conversation surface

The workbench SHALL be the only conversation surface. Selecting a session in the
rail SHALL focus it in place — the operator SHALL NOT be navigated away from the
workbench to a separate detail page. The `/sessions/{key}` deep link SHALL keep
resolving and SHALL render the same workbench with that session focused, so
bookmarks and links keep working. The rail's current-session marker SHALL follow
the focused-session pointer rather than the browser location. Every per-session
action the retired detail page offered — close, archive, and the gated-call
review cards — SHALL remain reachable from the workbench.

#### Scenario: selecting a session keeps the operator in the workbench

- **WHEN** the operator selects a session in the rail
- **THEN** that session becomes the focused one and the workbench renders its conversation without a page change to a different surface

#### Scenario: deep link renders the workbench

- **WHEN** a bookmarked `/sessions/{key}` is opened
- **THEN** the workbench renders with that session focused, rather than a separate detail page

#### Scenario: the rail marker follows focus

- **WHEN** the focused session changes through any supported path
- **THEN** the rail marks the focused session as current regardless of the browser location

#### Scenario: per-session actions stay reachable

- **WHEN** the operator focuses a session whose child is running
- **THEN** close, archive and that session's gated-call review cards are reachable from the workbench
