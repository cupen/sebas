## MODIFIED Requirements


### Requirement: Workbench renders the focused session as a conversation

The workbench SHALL render the focused session's transcript as a conversation: prompts, streaming text, thinking, tool calls with their decisions, notices, and errors, appended live as the turn progresses.

A thinking entry SHALL render its actual content: inside its process fold, the expanded (default) view SHALL show the thinking text carried by the entry — not a placeholder word standing in for it. An operator expanding a thinking fold SHALL be able to read what the model thought, matching the session detail entries for the same turn.

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
- **THEN** those invocations are rendered as second-level folds inside their process fold and are distinguishable from the turn's prose

#### Scenario: fold affordance is a lightweight link

- **WHEN** a process fold or a second-level fold renders in its collapsed state
- **THEN** its affordance is a single inline text control carrying the fold's summary, with no large button, border, or card chrome around it

#### Scenario: clicking the affordance toggles the fold

- **WHEN** the operator clicks a collapsed fold's link affordance
- **THEN** the fold expands in place to reveal its body
- **WHEN** the operator clicks the same affordance again
- **THEN** the fold returns to its collapsed summary

#### Scenario: long entry expands truncated with a view-all escape

- **WHEN** the operator expands a second-level entry whose content exceeds the truncation threshold
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

#### Scenario: thinking 折叠组展开显示实际内容

- **WHEN** a turn produces thinking entries (element_type `thinking` with non-empty content) and the operator views the process fold in the workbench
- **THEN** the fold shows the thinking text (e.g. the entry content) rather than a generic placeholder, matching the session detail API for the same entries


### Requirement: Desired and effective session mode are both visible

A session's desired permission mode and its effective mode SHALL both be visible in the workbench, and the composer mode control SHALL offer the session mode set (ask / edit / allow / auto) with the current desired mode selected.

When two offered mode values map to the same effective behavior on the session's backend (allow and auto both resolving to the agent's bypass level), the mode control SHALL state that equivalence in its option labeling or helper text, so the operator can distinguish the choices without reading the docs. The labeling SHALL NOT change which mode value is sent or how it gates approvals.

#### Scenario: unenforceable mode is shown as such

- **WHEN** a session's execution body cannot enforce the desired mode
- **THEN** the session shows both values and states that the desired mode is not
  enforced

#### Scenario: auto session is distinguishable

- **WHEN** a session runs in `auto` mode
- **THEN** the workbench marks it as ungated so an operator can tell it apart
  from a session that asks for decisions

#### Scenario: 等价模式有标注

- **WHEN** the operator opens the composer mode menu on a claude-driver session (where allow and auto both resolve to bypassPermissions)
- **THEN** the allow and auto options (or their helper text) state that both take effect identically on this backend


## ADDED Requirements

### Requirement: Dialog entrance never swallows the opening click

Dialog-opening interactions in the workbench (create-session, add-project, settings sections, new-agent form) SHALL tolerate a click that lands during the dialog's entrance: the click that opens a dialog SHALL NOT be re-interpreted as a backdrop dismissal, and the dialog's own controls SHALL accept clicks as soon as they are visible. A dialog that closes itself because the opening interaction's coordinates were still animating is a violation. An operator who clicks a visible section button inside an open dialog SHALL see that section activate; the dialog SHALL NOT silently ignore the click nor dismiss itself.

#### Scenario: 快速点击不误关对话框

- **WHEN** the operator clicks a settings entry (e.g. ＋新建 agent) and immediately clicks a visible control inside the opened dialog
- **THEN** the dialog stays open and the clicked control takes effect

#### Scenario: 分区按钮点击即生效

- **WHEN** the settings modal is open and the operator clicks a visible section button (e.g. 模型)
- **THEN** that section becomes active on the first click, without retrying
