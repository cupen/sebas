# agent-core delta — sebas-agent (Phase 1a)

## Purpose

Owns sebas's native in-process coding agent kernel (product name **sebas-agent**): per-session turn loops over an LLM reached through a configurable Anthropic Messages endpoint (a provider used directly, or an optional gateway as a routing layer), a uniform tool interface with a six-tool starter set (bash / read / write / edit / glob / grep), project-memory injection, and cancellation and budget semantics — emitting events in the existing `AcpEvent` vocabulary so current card and SSE consumers can render its sessions unchanged.

## ADDED Requirements

### Requirement: Session lifecycle and concurrency

The system SHALL support creating multiple concurrent agent sessions in one process, each bound to a working directory, accepting prompts and cancellation, and exposing a per-session event stream. Sessions SHALL live in the hosting process's memory; no persistence is required.

#### Scenario: Two sessions do not cross events

- **WHEN** two sessions are created with different working directories and both are prompted
- **THEN** each session's event stream contains only events for its own session id
- **AND** the tool executions of one session never observe the other's working directory

#### Scenario: Session stays usable after a cancelled turn

- **WHEN** a turn is cancelled and the operator prompts the same session again
- **THEN** the new turn executes normally on top of the preserved history

### Requirement: Turn loop

The system SHALL execute a prompt as a loop: send the conversation to the LLM, execute every tool call in the response, return the tool results as tool-result messages, and repeat — until the response contains no tool call, the turn budget is exhausted, the turn fails, or the turn is cancelled.

#### Scenario: Multi-step task completes without operator input

- **WHEN** the model responds across multiple rounds with tool calls (five or more tool executions) and finally without one
- **THEN** all tool results were fed back before the next model call
- **AND** the loop ends with a terminal finished event only after the tool-call-free response

### Requirement: Turn budgets

The system SHALL enforce per-turn limits on model calls, tool executions, and wall-clock time. Exhausting any limit SHALL end the turn as a normal finish carrying a budget-exhausted indication — not as an error.

#### Scenario: Model-call budget ends the turn cleanly

- **WHEN** the model-call limit is reached while the model still requests tools
- **THEN** no further model call is made
- **AND** the turn ends with a terminal finished event whose payload indicates budget exhaustion
- **AND** the session is not marked terminally failed

### Requirement: Cancellation safety

The system SHALL support cancelling a turn at any point. Cancelling SHALL abort the in-flight model request or terminate running tool processes, preserve the session history, and return the session to a state accepting the next prompt.

#### Scenario: Cancel during a long-running shell tool

- **WHEN** cancellation arrives while a shell tool is running
- **THEN** the tool's process group is terminated
- **AND** the session emits a cancellation outcome rather than a finished outcome
- **AND** no orphaned child process survives the cancellation

### Requirement: LLM channel

The system SHALL reach the LLM exclusively by speaking the Anthropic Messages streaming protocol to a configured endpoint authenticated with a configured credential. The endpoint SHALL be configurable: a provider endpoint used directly — the default path, requiring no gateway — or a gateway as an optional routing layer. The system SHALL NOT embed any provider SDK. Tool call arguments SHALL be assembled from incremental JSON fragments delivered by the stream before any tool executes.

#### Scenario: Tool arguments arrive as fragments

- **WHEN** a streamed response delivers a tool call's arguments as multiple incremental JSON fragments
- **THEN** no tool starts executing before the arguments assemble into valid JSON
- **AND** the executed tool receives the complete argument object

#### Scenario: Direct provider endpoint without a gateway

- **WHEN** the client is configured with a provider base URL and API credential directly
- **THEN** requests are sent to that endpoint and no gateway is contacted

### Requirement: Streaming event vocabulary

The system SHALL emit incremental events as they occur — text and thinking deltas while streaming, a tool-start event before each tool execution with its name and arguments, and a tool-end event with the result after it completes — using only the existing `AcpEvent` variants. The system SHALL NOT emit new event variants, and SHALL NOT emit permission-request events.

#### Scenario: Deltas arrive during streaming

- **WHEN** the model streams text or thinking content
- **THEN** corresponding delta events are emitted as each increment arrives, before the turn completes

### Requirement: Uniform tool interface with six-tool set

The system SHALL expose exactly six tools to the model — bash, read, write, edit, glob, grep — each declaring its name, usage description, and JSON-schema parameters. Tool outputs SHALL be size-capped with truncation indicated. File-modifying tools (write, edit) SHALL refuse to modify an existing file that was not read earlier in the same session.

#### Scenario: Write without prior read is refused

- **WHEN** the model calls write on a file that exists but was never read in this session
- **THEN** the tool returns a refusal carrying the reason, and the file is unchanged

#### Scenario: Edit with ambiguous match is refused

- **WHEN** the model calls edit and the old text matches zero or multiple locations without replace-all
- **THEN** the tool returns an error stating the actual number of matches, and the file is unchanged

#### Scenario: Glob is capped and ordered

- **WHEN** a glob pattern matches more than one hundred files
- **THEN** the result lists at most one hundred files ordered by modification time and is marked truncated

#### Scenario: Grep is capped

- **WHEN** a grep matches more than two hundred fifty lines
- **THEN** the result carries at most two hundred fifty line matches and is marked truncated

#### Scenario: Read supports paged re-reading

- **WHEN** the model reads a file with an offset and limit
- **THEN** the result contains the requested line range with line numbers

### Requirement: Tool failure semantics

The system SHALL return tool failures to the model as structured tool results instead of crashing or aborting the turn. A shell tool exiting nonzero SHALL be surfaced as a successful tool result carrying the exit code and output, so the model can observe and recover from command failure.

#### Scenario: Model recovers from a failed command

- **WHEN** a shell tool exits nonzero and the model is invoked again with that result
- **THEN** the loop continues normally with the failure visible to the model

#### Scenario: Tool timeout terminates the command

- **WHEN** a shell tool exceeds its time limit
- **THEN** the process group is terminated and the model receives a timeout failure result

### Requirement: Project memory injection

The system SHALL include in every session's system prompt the project's AGENTS.md followed by CLAUDE.md, each when present in the session's working directory, and SHALL proceed with the base prompt when neither exists.

#### Scenario: Memory files are injected

- **WHEN** a session's working directory contains AGENTS.md
- **THEN** the first model request of every turn carries its content in the system prompt
