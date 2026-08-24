## Purpose

Defines how the bot renders assistant output as interactive Feishu cards: the
per-turn card model, streaming update cadence, content layout, thinking/tool
rendering, long-content truncation and card rotation, interactive elements,
help cards, and error/status cards.

## ADDED Requirements

### Requirement: Per-turn card model

The router SHALL maintain exactly one `CardState` per session, seeded when the
session spawns and reset at the start of each user turn. Each user turn that
spawns or continues a session SHALL produce a fresh reply card that threads to
the user's input message (`root_id`); cards from earlier turns SHALL remain
frozen (no further updates). While a turn is in flight, additional user
messages SHALL NOT create a new card.

#### Scenario: first turn creates the card

- **WHEN** a session spawns for a user message and streaming events arrive
- **THEN** the router emits one `SendCard` whose `root_id` is the user's
  message id, and all subsequent updates for that turn PATCH the same card

#### Scenario: second turn creates a new card

- **WHEN** the user sends another message after a turn completes
- **THEN** the router resets the card body and emits a new `SendCard` threading
  to the new user message, leaving the previous turn's card untouched

#### Scenario: in-flight message does not create a card

- **WHEN** a user message arrives while a turn is still streaming
- **THEN** the message is enqueued and no new card is created for it

### Requirement: Card structure

Every turn card SHALL be structured top-to-bottom as: header (title derived
from the first non-empty line of the user prompt, truncated to 40 chars),
quote block containing the user prompt, divider, body elements, and footer.
The footer SHALL show the model and token usage as
`{model} · in: {input} out: {output} · ctx: {total_input}` when usage is
known, otherwise `msg_id: {session_id}`.

#### Scenario: usage footer

- **WHEN** a turn finishes with a usage event carrying input=10, output=25,
  total_input=12 for model `claude-x`
- **THEN** the card footer renders `claude-x · in: 10 out: 25 · ctx: 12`

#### Scenario: title truncation

- **WHEN** the user prompt's first non-empty line is 80 characters
- **THEN** the card header title shows only the first 40 characters

### Requirement: Streaming update cadence

Streaming events SHALL accumulate in an in-memory card body and flush as a
single `UpdateCard` per debounce tick (150 ms), coalescing multiple deltas
into one API call. The router SHALL flush immediately — bypassing the debounce
— on `Finished`, terminal `Error`, and `PermissionRequest` events.

#### Scenario: deltas coalesce

- **WHEN** five text deltas arrive within one debounce window
- **THEN** exactly one `UpdateCard` call flushes after the window

#### Scenario: terminal event flushes immediately

- **WHEN** the `Finished` event arrives mid-debounce-window
- **THEN** the pending body flushes without waiting for the window to expire

### Requirement: Terminal state rendering

When a turn finishes, the body's working panel label SHALL change from
`🤔 折腾中` to `✅ 已完成`. A terminal error SHALL append an `❌ {message}`
row to the body and finalize the card; the card itself SHALL remain visible
as the record of the failure.

#### Scenario: finished panel rename

- **WHEN** a turn transitions to `Finished`
- **THEN** the collapsible working panel's header text becomes `✅ 已完成`

#### Scenario: terminal error row

- **WHEN** the ACP session reports a terminal error with message `boom`
- **THEN** the card body ends with an `❌ boom` row and receives no further
  updates

### Requirement: Thinking display policy

When `card.thinking` is `show` (the default), adjacent thinking deltas SHALL
be folded into a collapsed `💭 思考` collapsible panel in the body; when long
output folding is enabled the panel nests inside the turn's working panel.
When `thinking` is `hide`, thinking deltas SHALL be dropped entirely.

#### Scenario: show folds thinking

- **WHEN** `thinking` is `show` and thinking deltas stream in
- **THEN** the body contains a collapsed `💭 思考` panel holding the thinking
  text, separate from the output text

#### Scenario: hide drops thinking

- **WHEN** `thinking` is `hide` and thinking deltas stream in
- **THEN** no panel or text for the thinking content appears in the card body

### Requirement: Long-content truncation and suppression

The card SHALL enforce content limits: user-visible text output is truncated
to `card.max_user_text_chars` (default 4000) when
`card.fold_long_output` is true, appending a `（已折叠 N 字）` note (UTF-8
safe truncation); when folding is false the full text is kept. Tool output
SHALL be suppressed entirely when `card.max_tool_output_chars` is 0 (the
default), rendered up to the configured size otherwise, with a hard cap of
10240 chars. Tool progress notes are capped at 5 per tool panel.

#### Scenario: text truncation

- **WHEN** `fold_long_output` is true and a text delta block of 9000 chars
  flushes with `max_user_text_chars` = 4000
- **THEN** the body shows the first 4000 chars plus `（已折叠 5000 字）`

#### Scenario: default tool output suppression

- **WHEN** a tool returns output and `max_tool_output_chars` is 0
- **THEN** the tool panel records the call without the output content

#### Scenario: tool output cap

- **WHEN** `max_tool_output_chars` is 2000 and a tool returns 8000 chars
- **THEN** at most 2000 chars are rendered, and no configuration value above
  10240 is honored

### Requirement: Body budget and card rotation

The card body SHALL enforce a budget of 24000 chars and 80 elements; when a
flush would exceed it, the oldest elements are dropped. When a turn's content
reaches 80% of the budget, the router SHALL rotate: freeze the current card,
seed a new card beginning with a `📎 接上条，内容继续` note, and continue
streaming into the new card.

#### Scenario: budget eviction

- **WHEN** appending a new element would push the body past 80 elements
- **THEN** the oldest element is evicted so the flush fits the budget

#### Scenario: rotation on long turn

- **WHEN** a single turn accumulates content beyond 80% of the body budget
- **THEN** the current card is finalized and a new continuation card is
  created with the `📎 接上条，内容继续` prefix note

### Requirement: Interactive card JSON

Cards SHALL be emitted as Feishu card schema `2.0` JSON using interactive v2
elements. The element vocabulary used by the system comprises: `hr`,
`markdown`, `div` (text), `div` (fields), `button`, `collapsible_panel`,
`form`, `select_static`, and `column_set`. Buttons SHALL be expressed as
first-class v2 buttons, not V1 action containers.

#### Scenario: v2 button rendering

- **WHEN** a permission card with three decision buttons is rendered
- **THEN** the card JSON contains v2 `button` elements and the card is
  accepted by the Feishu card API (no V1 `action` block)

#### Scenario: client version constraint

- **WHEN** a card containing `collapsible_panel` is sent
- **THEN** panels render only on Feishu clients at or above the v7.9 version
  that introduced collapsible panels

### Requirement: Help card

The `/help` command SHALL render an interactive help card organized as tabs
(命令 / 会话 / 管理 / 通道), with command buttons laid out in 2–3 columns
via `column_set` (wide commands taking a full row). Tab switching SHALL
update the same card in place (PATCH by msg id) rather than sending a new
card. Clicking a command button SHALL behave as if the user typed that
command's text.

#### Scenario: tab switch in place

- **WHEN** the user clicks the 会话 tab button on the help card
- **THEN** the existing help card is updated in place to show session
  commands, with no new message in the chat

#### Scenario: command button invocation

- **WHEN** the user clicks a command button on the help card
- **THEN** the router processes the corresponding command text through the
  same path as a typed message

### Requirement: Error and status cards

System events SHALL be reported as dedicated cards: spawn failure as a red
`❌ 启动失败` card (detail in a code fence when multi-line or over 120 chars);
interaction with a dead session as a grey `会话已结束` card; a rejected
resume falling back to a fresh session as an orange `已开启新会话` card.

#### Scenario: spawn failure card

- **WHEN** the ACP child fails to start with error `claude not found`
- **THEN** the bot sends a red card titled `❌ 启动失败` containing the error

#### Scenario: dead session interaction

- **WHEN** a button callback arrives for a session whose mapping is gone
- **THEN** the bot replies with a grey `会话已结束` card instead of routing
  the action

### Requirement: Card lifecycle cleanup

The router SHALL drop the card state when a session ends (terminal error,
channel close, or explicit close), so subsequent interaction cannot update a
stale card. Message-id mappings are overwritten per turn.

#### Scenario: card state dropped on session end

- **WHEN** a session terminates with a terminal error
- **THEN** the card state is removed and later updates for that session id
  are discarded

### Requirement: Card theme configuration

The card theme color (`card.theme_color`, default `blue`) SHALL flow into the
card header template. Card settings SHALL be parsed with strict
(deny-unknown-fields) semantics — an unknown key in `[card]` is a
configuration error rather than a silent ignore — and persisted as a
full-snapshot JSON file written atomically with mode 0600.

#### Scenario: default theme

- **WHEN** no `[card]` section is configured
- **THEN** card headers render with the blue template

#### Scenario: unknown card key rejected

- **WHEN** the config file contains `[card]` with key `theme_colr`
- **THEN** configuration parsing fails with an unknown-field error
