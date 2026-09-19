## MODIFIED Requirements

### Requirement: Workbench is the single conversation surface

The workbench SHALL be the only conversation surface. Selecting a session in the
rail SHALL focus it in place — the operator SHALL NOT be navigated away from the
workbench to a separate detail page. The `/sessions/{key}` deep link SHALL keep
resolving and SHALL render the same workbench with that session focused, so
bookmarks and links keep working. The rail's current-session marker SHALL follow
the focused-session pointer rather than the browser location. Every per-session
action the retired detail page offered — close, archive, and the gated-call
review cards — SHALL remain reachable from the workbench. Selecting a session in
the rail SHALL take effect immediately: the workbench MUST render the selected
session's conversation without waiting for an unrelated session event to refresh
the focus pointer.

#### Scenario: selecting a session keeps the operator in the workbench

- **WHEN** the operator selects a session in the rail
- **THEN** that session becomes the focused one and the workbench renders its conversation without a page change to a different surface

#### Scenario: rail selection renders the conversation immediately

- **WHEN** the operator selects a session in the rail while a different session is displayed, and no other session event occurs
- **THEN** the workbench renders the selected session's conversation within the focus-follow latency of an ordinary session event (no dependence on subsequent unrelated events)

#### Scenario: deep link renders the workbench

- **WHEN** a bookmarked `/sessions/{key}` is opened
- **THEN** the workbench renders with that session focused, rather than a separate detail page

#### Scenario: the rail marker follows focus

- **WHEN** the focused session changes through any supported path
- **THEN** the rail marks the focused session as current regardless of the browser location

#### Scenario: per-session actions stay reachable

- **WHEN** the operator focuses a session whose child is running
- **THEN** close, archive and that session's gated-call review cards are reachable from the workbench

### Requirement: Workbench renders the focused session as a conversation

The workbench SHALL render the focused session as a conversation between the
operator and the agent, in transcript order: each submission the operator made
SHALL appear as their own turn, rendered as a lightly tinted block without
card chrome (no border or shadow), and each agent turn SHALL render as a
natural conversation flow — the author label followed by the turn's content
laid out directly in the conversation, not wrapped in a large bubble or card
container. One agent turn SHALL be composed of everything the agent produced
for that turn — the streamed text concatenated in arrival order and its
thinking and tool invocations — and SHALL NOT be rendered as a series of
per-chunk bubbles. Within an agent turn, entries SHALL be split in arrival
order into alternating text runs and process runs: every run of contiguous
thinking/tool entries SHALL become ONE collapsed-by-default process fold
positioned between the text segments at the run's actual position in the
turn (replacing the former single turn-wide fold); each fold SHALL contain
second-level folds — one per thinking segment and one per tool invocation —
each collapsed by default, titled per the structured-title rule. Process
folds SHALL stay collapsed while their entries stream in, and the fold's
summary row SHALL update live during streaming (the running tool's title and
the entry count); when the operator has expanded a fold, newly streamed
entries of that fold SHALL append in place. A submission SHALL appear in the
conversation only when its turn starts. A turn that terminates in an engine
error — including a refusal result or any `is_error` terminal — SHALL render
a visible error entry in the conversation naming the failure; the operator
SHALL NEVER see a submitted message followed by silence with no agent-side
entry. The error entry's summary label SHALL state the actual failure class
(such as spawn failure or turn stall) rather than a fixed generic string.

#### Scenario: both sides of the conversation are visible

- **WHEN** the operator opens a session in which they submitted messages across several turns
- **THEN** the workbench shows their submissions and the agent's replies in transcript order, each submission as the operator's own tinted block

#### Scenario: one agent turn is one bubble

- **WHEN** an agent turn arrives as many streamed text chunks plus thinking plus tool invocations
- **THEN** the workbench renders that turn as one continuous natural flow — the text in order with all thinking and tool entries interleaved as collapsed process folds at their positions — and not as a series of per-chunk bubbles nor as a card-wrapped block

#### Scenario: process folds interleave in arrival order

- **WHEN** an agent turn produces text, then a run of thinking and tool entries, then more text
- **THEN** one collapsed process fold sits between the two text segments, at the position where those entries occurred

#### Scenario: folds stay collapsed with a live summary while streaming

- **WHEN** thinking and tool entries stream into a process run while its fold is collapsed
- **THEN** the fold does not open by itself, and its summary row updates live to reflect the running tool and the accumulated entry count

#### Scenario: an expanded fold appends streamed entries in place

- **WHEN** the operator has expanded a process fold while its turn is still streaming
- **THEN** new entries of that run appear inside the fold as they arrive, without collapsing it

#### Scenario: tool invocations are not mistaken for prose

- **WHEN** an agent turn invokes tools
- **THEN** those invocations are rendered as second-level folds inside their process fold and are distinguishable from the turn's prose

#### Scenario: a submission appears when its turn starts

- **WHEN** a submission is accepted while the agent is still working
- **THEN** it is not rendered as a started turn until its turn actually begins

#### Scenario: a refused turn renders an error entry

- **WHEN** an agent turn ends with a refusal or other `is_error` result that produced no text entries
- **THEN** the conversation shows a visible error entry for that turn following the operator's submission, and the session remains usable for the next submission

#### Scenario: error entries name their failure class

- **WHEN** a turn is force-settled by the stall watchdog
- **THEN** its error entry's summary label identifies the stall (not a generic spawn-failure label), while a genuine spawn failure is labeled as such

#### Scenario: a denied tool result shows a denied marker

- **WHEN** an expanded process fold contains a denied tool result
- **THEN** the denied entry's detail is prefixed with a denied marker consistent with its collapsed title, not an approved marker
