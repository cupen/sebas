## ADDED Requirements

### Requirement: Rail project order is operator-controlled

项目栏 SHALL 提供重排序入口（每个项目条目的菜单或拖拽任选其一），操作员调整的顺序 SHALL 经既有项目重排序端点持久化，刷新后保持。未手动排序时维持既有缺省序。重排序只影响呈现顺序，不改变任何会话路由或焦点指针。

#### Scenario: reorder entry exists

- **WHEN** 操作者打开项目条目的上下文菜单（或拖拽条目）
- **THEN** 存在可用的重排序操作（上移/下移或拖拽），操作后条目顺序即时更新

#### Scenario: reorder persists across reload

- **WHEN** 操作者调整项目顺序后刷新页面
- **THEN** 项目栏保持调整后的顺序

#### Scenario: reorder is presentation-only

- **WHEN** 任一项目被移动
- **THEN** 该项目的会话、焦点指针与消息路由均不受影响

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
positioned between the text segments at the run's actual position in the turn
(replacing the former single turn-wide fold); each fold SHALL contain
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

Run 分派 SHALL 如实：正文条目 SHALL 归入文本段——文本段 SHALL NOT 携带任何
过程芯片/折叠提示（「PROCESS …」类标签只出现在过程折叠上）；thinking 段的
第二级折叠标题 SHALL 标明 thinking。过程折叠的缺省（收起）呈现 SHALL 与
正文可区分——操作者无需展开即可从折叠行看出「此处有 thinking/工具过程」，
不得呈现为与正文段落无异的纯文本。

Text and thinking deltas that arrive while the turn is in flight SHALL render
into that turn's text run as they arrive — the operator SHALL see partial
agent output while the turn is still running, and SHALL NOT have to wait for
the turn's terminal phase before any of it appears. This live rendering SHALL
be observable in the browser DOM of the focused conversation, not merely in
the server-side transcript.

The collapsed affordance of every process fold and second-level fold SHALL be
a lightweight inline text control — a link-style toggle carrying the fold's
summary (label and count) — and SHALL NOT be rendered as a large button,
bordered block, or card chrome. Clicking the affordance SHALL expand that fold
in place; clicking it again SHALL collapse it.

When the operator expands a second-level entry whose content exceeds the
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
- **THEN** those invocations are rendered as second-level folds inside their process fold and are distinguishable from the turn's prose

#### Scenario: text never wears a process chip

- **WHEN** an agent turn renders its text segments (含 thinking 与正文交错的回合：thinking 前置、正文在后)
- **THEN** 正文段不携带任何「PROCESS …」类过程芯片或折叠提示；过程芯片只出现在过程折叠的收起行与第二级折叠上

#### Scenario: thinking fold is titled and distinguishable

- **WHEN** 一个含 thinking 段的回合按缺省（收起）形态呈现
- **THEN** thinking 段在过程折叠内以标明 thinking 的第二级折叠存在，其收起行与正文段落形态可区分（操作者不展开即可识别此处有 thinking）

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

### Requirement: History group is the archive

The History group SHALL contain only archived sessions, listed newest-first by archive time. Each archived entry SHALL present the operator label that was current at archive time（改名后归档的会话 SHALL 显示新名，不得回退到更早的自动标题）. The rail SHALL render one group per registered project and SHALL NOT render an Inbox group: every session the webui presents belongs to a project, and the only project-less sessions are Feishu-originated ones, which remain out of the rail entirely and are presented by their originating surface. The History group SHALL show the total count of archived sessions and be collapsible.

#### Scenario: History holds only archived sessions

- **WHEN** the History group is expanded
- **THEN** every session listed has been archived

#### Scenario: History is sorted newest-first

- **WHEN** sessions are archived at different times
- **THEN** the History group lists them in descending order of archive time

#### Scenario: History shows the current label at archive time

- **WHEN** a session is renamed and then archived
- **THEN** the History entry shows the renamed label, not an earlier auto title

#### Scenario: restore keeps the label

- **WHEN** a renamed session is restored from History
- **THEN** it returns to the rail with the same renamed label

#### Scenario: Inbox for unbound sessions

- **WHEN** a session has no project directory
- **THEN** the only such sessions are Feishu-originated ones: it appears in no rail group — the Inbox group does not exist — and History does not list it either; every webui-created session belongs to a project and is listed under it
