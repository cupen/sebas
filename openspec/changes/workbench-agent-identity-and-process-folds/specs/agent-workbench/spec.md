## MODIFIED Requirements

### Requirement: Workbench renders the focused session as a conversation

The workbench SHALL render the focused session as a conversation between the
operator and the agent, in transcript order: each submission the operator made
SHALL appear as their own turn, and each agent turn SHALL appear as a single
assistant bubble. One agent turn SHALL be composed of everything the agent
produced for that turn — the streamed text concatenated in arrival order, its
thinking, and its tool invocations — and SHALL NOT be rendered as a series of
per-chunk bubbles. All thinking and tool entries of the turn SHALL be
collected into ONE collapsed-by-default process fold inside the turn's bubble
(replacing the former per-run thinking folds and the separate "used N tools"
group); the fold SHALL sit at the position of the turn's first process entry,
with the streamed text segments remaining outside it in arrival order.
Expanding the process fold SHALL reveal second-level folds — one per thinking
segment and one per tool invocation — each collapsed by default. A submission
SHALL appear in the conversation only when its turn starts.

#### Scenario: both sides of the conversation are visible

- **WHEN** the operator opens a session in which they submitted messages across several turns
- **THEN** the workbench shows their submissions and the agent's replies in transcript order, each submission as the operator's own turn

#### Scenario: one agent turn is one bubble

- **WHEN** an agent turn arrives as many streamed text chunks plus thinking plus tool invocations
- **THEN** the workbench renders one assistant bubble for that turn, with the text in order and all thinking and tool entries collected into a single collapsed process fold inside the bubble

#### Scenario: tool invocations are not mistaken for prose

- **WHEN** an agent turn invokes tools
- **THEN** those invocations are rendered as second-level folds inside the process fold and are distinguishable from the turn's prose

#### Scenario: a submission appears when its turn starts

- **WHEN** a submission is accepted while the agent is still working
- **THEN** it is not rendered as a started turn until its turn actually begins

## ADDED Requirements

### Requirement: Assistant turn identity shows the bound agent

The author identity of each assistant turn SHALL display the display name of
the agent kind bound to the session (e.g. `Claude Code`), resolved from the
agent catalog (`/api/agents`) by the session's `agent_kind`; the reserved
`native` kind SHALL display its catalog name. When no display name is
available the raw slug SHALL be shown. The avatar remains text-form (icon
graphics are out of scope).

#### Scenario: agent display name is shown

- **WHEN** the focused session is bound to an agent kind whose catalog
  display name is `Claude Code`
- **THEN** the assistant turns' author label reads `Claude Code` rather than
  a generic `assistant`

#### Scenario: missing display falls back to slug

- **WHEN** the agent kind has no display name in the catalog
- **THEN** the author label falls back to the raw `agent_kind` slug

### Requirement: Operator submission receipt

After the operator's submission is accepted by the server (the entry is
present in the session payload) and before the agent's output for that turn
begins arriving, the operator's message bubble SHALL show a low-key receipt
badge indicating the message was received. The badge SHALL disappear once
entries of the agent's reply turn start arriving. The badge is a visual hint
only — it SHALL NOT change turn ordering or persistence semantics.

#### Scenario: badge shows while waiting

- **WHEN** the operator sends a message and the server has accepted it, but
  no agent output entries for the reply turn have arrived yet
- **THEN** the operator's message bubble shows the receipt badge

#### Scenario: badge clears when the reply streams

- **WHEN** the first entry of the agent's reply turn arrives
- **THEN** the receipt badge on the operator's message disappears

### Requirement: Process fold titles summarize entries

Tool entries SHALL carry an optional structured `title` on the wire
（`TurnEntry` 增可选字段，缺省 None，旧持久化条目无需迁移）, built by the
backend as the tool name plus its key argument (e.g. the path a `read` call
reads). Each second-level fold's collapsed title SHALL show the entry's
`title` when present; thinking folds show a generic stable label. Titles
longer than the title area SHALL be middle-truncated（保留首尾、中部省略号）.
Entries without a title SHALL fall back to a generic label.

#### Scenario: tool fold shows name and path

- **WHEN** a `read` tool call entry with title `read · src/main.rs` renders
  inside the process fold
- **THEN** its second-level fold's collapsed title shows the tool name and
  the path

#### Scenario: long title is middle-truncated

- **WHEN** an entry title is longer than the fold title area allows
- **THEN** the title is displayed with head and tail preserved and the
  middle replaced by an ellipsis marker

#### Scenario: legacy entries fall back

- **WHEN** a persisted tool entry predates the `title` field (absent/None)
- **THEN** its fold falls back to a generic label (e.g. the tool count-free
  form) without errors
