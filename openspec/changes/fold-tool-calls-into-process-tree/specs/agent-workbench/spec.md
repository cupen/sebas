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
sub-folds — one per thinking segment and one per tool call — each collapsed
by default, titled per the structured-title rule. A tool call's invocation and
its result SHALL render as ONE sub-fold inside the process fold (see
"Tool call blocks merge invocation and result"); there SHALL be no
top-level tool-result block rendered outside the process fold. Process
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

Text and thinking deltas that arrive while the turn is in flight SHALL render
into that turn's text run as they arrive — the operator SHALL see partial
agent output while the turn is still running, and SHALL NOT have to wait for
the turn's terminal phase before any of it appears. This live rendering SHALL
be observable in the browser DOM of the focused conversation, not merely in
the server-side transcript.

The collapsed affordance of every process fold and sub-fold SHALL be
a lightweight inline text control — a link-style toggle carrying the fold's
summary (label and count) — and SHALL NOT be rendered as a large button,
bordered block, or card chrome. Clicking the affordance SHALL expand that fold
in place; clicking it again SHALL collapse it. Toggling a sub-fold SHALL NOT
collapse, or otherwise alter the open state of, the process fold that contains
it — no nesting level may make another level's content unreachable.

When the operator expands a sub-fold whose content exceeds the
presentation's truncation threshold, the body SHALL render the truncated
portion together with an explicit truncation notice stating how much was
omitted, plus a "view all" control. The "view all" control SHALL open the
entry's full content in an isolated overlay outside the conversation's scroll
container; closing the overlay SHALL remove its content from the DOM so that
the conversation's scroll surface gains no lasting nodes.

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

#### Scenario: streamed deltas appear while the turn is still running

- **WHEN** the agent's turn emits text deltas spaced in time and the session is still in its running phase
- **THEN** the deltas already rendered into the turn's text run are visible in the conversation at that moment, before the turn reaches its terminal phase

#### Scenario: an expanded fold appends streamed entries in place

- **WHEN** the operator has expanded a process fold while its turn is still streaming
- **THEN** new entries of that run appear inside the fold as they arrive, without collapsing it

#### Scenario: tool invocations are not mistaken for prose

- **WHEN** an agent turn invokes tools
- **THEN** those invocations are rendered as sub-folds inside their process fold and are distinguishable from the turn's prose

#### Scenario: no tool result renders outside the process fold

- **WHEN** an agent turn completes a tool call whose result entry has landed
- **THEN** the result is reachable inside that call's sub-fold within the process fold, and no separate top-level result block is rendered alongside the process fold

#### Scenario: fold affordance is a lightweight link

- **WHEN** a process fold or a sub-fold renders in its collapsed state
- **THEN** its affordance is a single inline text control carrying the fold's summary, with no large button, border, or card chrome around it

#### Scenario: clicking the affordance toggles the fold

- **WHEN** the operator clicks a collapsed fold's link affordance
- **THEN** the fold expands in place to reveal its body
- **WHEN** the operator clicks the same affordance again
- **THEN** the fold returns to its collapsed summary

#### Scenario: toggling a sub-fold leaves its parent open

- **WHEN** the operator expands a process fold and then expands and collapses a sub-fold inside it
- **THEN** the process fold stays expanded throughout, and the sub-fold's own body is reachable without re-expanding any ancestor

#### Scenario: long entry expands truncated with a view-all escape

- **WHEN** the operator expands a sub-fold whose content exceeds the truncation threshold
- **THEN** the body shows the truncated portion with a notice stating how much was omitted, and a "view all" control

#### Scenario: view-all overlay is isolated from the conversation scroll

- **WHEN** the operator activates "view all" on a truncated entry
- **THEN** the full content opens in an overlay outside the conversation's scroll container, and closing it removes that content from the DOM without changing the conversation's scroll layout

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

### Requirement: Process fold titles summarize entries

Tool entries SHALL carry an optional structured `title` on the wire
（`TurnEntry` 增可选字段，缺省 None，旧持久化条目无需迁移）, built by the
backend as the tool name plus its key argument (e.g. the path a `read` call
reads). Tool entries SHALL likewise carry an optional `tool_use_id` on the
wire, taken from the upstream tool-call identifier, so that a tool call's
invocation and result entries can be paired without positional guessing.
Each sub-fold's collapsed title SHALL show the entry's `title` when present;
thinking folds show a generic stable label. Titles
longer than the title area SHALL be middle-truncated（保留首尾、中部省略号）.
Entries without a title SHALL fall back to a generic label.

#### Scenario: tool fold shows name and path

- **WHEN** a `read` tool call entry with title `read · src/main.rs` renders
  inside the process fold
- **THEN** its sub-fold's collapsed title shows the tool name and
  the path

#### Scenario: long title is middle-truncated

- **WHEN** an entry title is longer than the fold title area allows
- **THEN** the title is displayed with head and tail preserved and the
  middle replaced by an ellipsis marker

#### Scenario: legacy entries fall back

- **WHEN** a persisted tool entry predates the `title` field (absent/None)
- **THEN** its fold falls back to a generic label (e.g. the tool count-free
  form) without errors

#### Scenario: tool call entries carry their upstream call id

- **WHEN** a tool invocation is projected into the transcript
- **THEN** its entry carries the upstream `tool_use_id`, and the paired result entry carries the same id

## ADDED Requirements

### Requirement: Tool call blocks merge invocation and result

A tool invocation and its result SHALL be presented as ONE collapsed-by-default
tool call block (a sub-fold of the process fold), not as two independent
blocks. The two transcript entries SHALL be paired by their `tool_use_id`;
pairing SHALL NOT rely on arrival position or tool name, so that concurrent
calls — including repeated calls to the same tool — pair correctly. The
block's collapsed title SHALL show the tool name plus its key argument
(`Read · src/main.rs`, `Grep · TODO`, `Bash · cargo test`) and SHALL carry the
call's outcome marker (已执行 / 已拒绝) so the outcome is readable without
expanding. Expanding the block SHALL reveal both the invocation's arguments
and the result content, each subject to the truncation and view-all rules.

An invocation whose result never arrived — the turn was interrupted, the call
failed, or the transcript was truncated — SHALL still render as its own
collapsed tool call block carrying the invocation-state title (no outcome
marker), so that no invocation and no argument detail is dropped.

#### Scenario: invocation and result render as one block

- **WHEN** a tool call's invocation entry and its result entry both exist in the transcript
- **THEN** the turn renders one collapsed tool call block for that call, not two blocks

#### Scenario: collapsed title carries the key argument

- **WHEN** a tool call block is collapsed
- **THEN** its title shows the tool name and its key argument, so the operator can tell what the call targeted without expanding it

#### Scenario: expanding reveals arguments and result

- **WHEN** the operator expands a tool call block
- **THEN** both the invocation's arguments and the result content are rendered in the block's body

#### Scenario: concurrent calls to the same tool pair correctly

- **WHEN** one turn issues two concurrent calls to the same tool with different arguments
- **THEN** each result is paired with its own invocation by `tool_use_id`, so each block shows the arguments belonging to its own call

#### Scenario: an unpaired invocation still renders

- **WHEN** an invocation has no result entry in the transcript
- **THEN** it still renders as its own collapsed tool call block with the invocation-state title and its arguments available on expansion

#### Scenario: outcome marker is visible while collapsed

- **WHEN** a tool call block whose call was denied renders collapsed
- **THEN** its title carries the denied marker without requiring expansion
