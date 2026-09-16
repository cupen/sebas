## MODIFIED Requirements

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
conversation only when its turn starts.

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

### Requirement: Operator submission receipt

After the operator's submission is accepted by the server (the entry is
present in the session payload) and before the agent's output for that turn
begins arriving, the operator's message SHALL show a low-key receipt badge
indicating the message was received. The badge SHALL disappear once entries
of the agent's reply turn start arriving. The badge is a visual hint only —
it SHALL NOT change turn ordering or persistence semantics.

#### Scenario: badge shows while waiting

- **WHEN** the operator sends a message and the server has accepted it, but no agent output entries for the reply turn have arrived yet
- **THEN** the operator's message block shows the receipt badge

#### Scenario: badge clears when the reply streams

- **WHEN** the first entry of the agent's reply turn arrives
- **THEN** the receipt badge on the operator's message disappears
