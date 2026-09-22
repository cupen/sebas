# session-slash-commands Specification

## Purpose
Lets the operator invoke agent-native slash commands (`/goal`, `/compact`, custom commands) from the workbench composer: the session's real command list is discovered from the agent (never hardcoded), rendered as an incrementally-filtered command palette above the composer, and submitted verbatim as ordinary prompt text — with unsupported commands intercepted honestly instead of silently swallowed by the agent.

## Requirements

### Requirement: Session command list is discovered from the agent

For each agent-backed session, the system SHALL surface the set of slash commands advertised by the agent. The claude path SHALL take the commands from the CLI initialization handshake (`get_server_info()` → `commands`, including description and argument hint when present); the generic ACP path SHALL parse `session/update` notifications carrying `available_commands_update`. The driver SHALL translate both intakes into the stable event vocabulary (`AcpEvent::AvailableCommands`), and the engine SHALL materialize the list into the session snapshot (per-session state, delivered to clients via the existing session payload and update events — no new API endpoint). The source SHALL be the agent's own advertisement — never a hardcoded list. When the agent advertises nothing (native engine, older agents), the session's command list SHALL be empty and consumers SHALL treat that as "no command surface", not an error. Re-advertisement (e.g. commands discovered mid-session) SHALL refresh the snapshot.

#### Scenario: claude session advertises built-in commands

- **WHEN** a claude-backed session is created and the CLI init handshake reports its commands (含 `goal`、`compact` 的描述与参数提示)
- **THEN** the session snapshot carries that command list and the webui session payload exposes it

#### Scenario: generic ACP agent advertises commands

- **WHEN** an opencode-backed session receives `available_commands_update` from the agent
- **THEN** the driver emits the command list event, the snapshot refreshes, and the webui payload exposes it

#### Scenario: no discovery capability degrades honestly

- **WHEN** a session runs on an agent that advertises no command list（如 native 引擎）
- **THEN** the session snapshot's command list is empty, the composer renders no command palette, and a leading `/` in a submitted message is treated as ordinary prompt text

### Requirement: Composer renders an incrementally-filtered command palette

When the composer input's first character is `/`, the composer SHALL render a command palette above the input listing the session's advertised commands — each row showing the command name and its argument hint when available, kept to a single compact line, filtered incrementally (prefix match, re-evaluated as the operator types). A command's description SHALL NOT be rendered inline in the row: it SHALL be presented in a floating detail bubble that appears when the row is hovered or made the keyboard highlight target. The bubble SHALL render the description as sanitized markdown, SHALL be bounded in size (width and height caps well below the viewport) and scroll internally when the description exceeds those bounds, and SHALL disappear when the hover or highlight moves away, the palette closes, or the session's command surface is absent. Keyboard palette navigation (↑/↓) SHALL keep the bubble anchored to the highlighted row with the same immediacy as hover, so the palette is fully usable without a pointer. The palette SHALL support ↑/↓ selection, Esc to dismiss, and a two-phase Enter/Tab: with a highlighted entry, the first Enter/Tab completes the command (inserts `name + space`, keeps focus in the input for arguments) and a subsequent Enter submits; Enter with no highlighted entry submits the raw text directly. The palette SHALL NOT render for sessions without a command surface.

#### Scenario: palette filters as the operator types

- **WHEN** the operator types `/`, then continues with `co` in a session advertising `compact` and `goal`
- **THEN** the palette appears on `/` listing both commands and narrows to `compact` as `co` is typed

#### Scenario: description renders only in the hover bubble

- **WHEN** the command palette is open and the operator hovers a command row
- **THEN** the row itself shows only the command name and argument hint, and the description appears as a bounded, internally scrollable markdown bubble anchored to that row

#### Scenario: keyboard highlight shows the same bubble

- **WHEN** the palette is navigated with ↑/↓ so that a row becomes the keyboard highlight target
- **THEN** the detail bubble appears for that row exactly as it would on hover, and moving the highlight moves the bubble

#### Scenario: oversized description scrolls inside the bubble

- **WHEN** a command's markdown description exceeds the bubble's size caps
- **THEN** the bubble scrolls internally, never growing beyond its caps, and its content is sanitized before rendering

#### Scenario: two-phase completion

- **WHEN** the operator highlights `/goal` and presses Tab (or Enter)
- **THEN** the input becomes `/goal ` with the palette dismissed and focus retained; a subsequent Enter submits the message

#### Scenario: palette absent without command surface

- **WHEN** the focused session has an empty command list and the operator types `/`
- **THEN** no palette renders and the text stays ordinary input

### Requirement: Slash commands pass through verbatim; unsupported ones are intercepted

A composer submission beginning with `/` SHALL be delivered to the agent unchanged through the existing message path (including busy/queued states) — the system SHALL NOT translate, expand, or locally execute it; the agent owns the command semantics and its response renders in the transcript like any turn. Before submitting a `/`-prefixed message to a session with a command surface, the composer SHALL validate the command name against the session's advertised list plus the universal built-in `compact` (advertised by claude, handled by opencode as an undocumented special case); a command outside that set SHALL be blocked with an inline notice naming the unsupported command and prompting re-input — it SHALL NOT be sent (agents like opencode swallow unknown commands silently as empty turns). Sessions without a command surface (empty list) SHALL NOT validate or block `/`-prefixed input.

#### Scenario: command reaches the agent verbatim

- **WHEN** the operator submits `/goal keep tests green until CI passes` in a claude session
- **THEN** the message is delivered unchanged as the turn prompt and the agent's response streams into the transcript

#### Scenario: unsupported command is intercepted

- **WHEN** the operator submits `/goal ...` in an opencode session whose advertised list does not contain `goal`
- **THEN** the composer blocks submission with an inline notice naming `goal` as unsupported by this session's agent, prompting re-input; no message is sent

#### Scenario: universal built-in passes without advertisement

- **WHEN** the operator submits `/compact` in an opencode session (which handles `compact` but does not advertise it)
- **THEN** the submission is delivered verbatim and the compaction result (or its silent completion) renders honestly in the transcript

#### Scenario: busy submission queues like a normal message

- **WHEN** the operator submits a slash command while the session's turn is in flight
- **THEN** the command follows the existing queued-submission path and executes after the current turn
