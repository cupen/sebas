# agent-core Specification

## Purpose

Extends the sebas-agent kernel (Phase 1a: per-session turn loops, six-tool set, budget, cancellation) into a usable coding agent: a policy engine that gates destructive and networked operations with explicit human approval and fail-closed defaults (checklist C3), a network tool surface for external research, context management (tool-result rewriting, assembly budgets, concurrent tool execution), optional multimodal and language-server capabilities with honest capability declaration, and the event vocabulary to drive the webui as its first approval answerer.

## Requirements

### Requirement: Policy-gated tool execution

The system SHALL check every tool execution against a policy before it runs. The policy SHALL default to deny when no rule matches and no approval answerer is reachable (fail-closed). Writes and networked operations SHALL be governed by a configurable policy; read-only operations on the session workdir SHALL be allowed by default. The exact-session allowlist SHALL be consulted before any interactive approval: an exact `(tool, args)` match runs silently, a partial signature match transitions to approval and, when approved for the session, upgrades to the allowlist.

#### Scenario: First networked write is denied without approval

- **WHEN** an unapproved `bash` command that writes outside the workdir is about to execute under `write` policy
- **THEN** the tool emits a permission request and does not execute until a decision returns
- **AND** if no answerer is reachable within the timeout, the request resolves as denied and the tool result reports the policy denial

#### Scenario: Allow-once approves one call with a stated reason

- **WHEN** the operator answers a permission request with `allow_on_use` and a reason
- **THEN** the exact `(tool, args)` call executes once with the reason attached to the request
- **AND** no allowlist entry is added

#### Scenario: Allow-session upgrades the exact signature

- **WHEN** the operator answers with `allow_on_session`, after the tool covered by the request runs
- **THEN** the exact `(tool, args)` signature joins the session allowlist and subsequent identical calls run without prompting
- **AND** a different tool or different arguments still prompt

#### Scenario: Explicit deny blocks the call and reachable policy keeps the session healthy

- **WHEN** the operator answers with `deny`
- **THEN** the tool returns a structured denial result, the call never runs, and the turn continues normally

### Requirement: One-shot escalation retry

Subject to operator approval, an attempted operation SHALL be able to retry once with a stated reason under an elevated policy, where the elevation SHALL apply to that retry call only and SHALL NOT widen the session policy.

#### Scenario: Approved escalation retries the same operation once

- **WHEN** an operation is denied by policy and the operator approves an escalation for it
- **THEN** the exact operation retries with the escalation's reason attached and the session policy is unchanged afterward

### Requirement: Policy error separation

The system SHALL emit, for every policy-denied or policy-failed execution, a distinguished policy event on the tool stream with a stable identity, so consumers can distinguish "denied by policy" from "tool crashed" and the turn continues normally after a denial.

#### Scenario: Policy denial appears as an event, not a crash

- **WHEN** a tool call is denied by policy
- **THEN** a `tool_policy` event with the request id, tool, and outcome is emitted on the session stream
- **AND** the agent loop continues with the next step

### Requirement: Sandboxed bash with graceful degradation

The system SHALL confine bash subprocesses by default to a workspace-write profile: writes permitted only inside the session workdir and the platform temporary directory, all TCP bind and connect denied, reads unrestricted. The confinement SHALL be enforced in-process by kernel-level file and network restrictions — no external binary, no privilege — and when the host cannot enforce that profile, the system SHALL fall back to a firewall profile (environment scrubbing of known secret patterns and literal probes refusing known dangerous binaries) rather than running unconstrained. The enforcing profile SHALL be reported honestly with each result, SHALL be deterministic per command, and SHALL never split a single command across profiles.

#### Scenario: Workspace escape is denied under the default profile

- **WHEN** bash writes outside the session workdir and platform tmp while the default profile is enforced
- **THEN** the write fails with a permission denial visible to the model
- **AND** the result is annotated with the enforcing profile

#### Scenario: Network is denied by default

- **WHEN** bash attempts a TCP connect while the default profile is enforced
- **THEN** the connect fails with a permission denial
- **AND** an unrestricted control run of the same command distinguishes the denial from an ordinary connection refusal

#### Scenario: Unsupported host degrades honestly to the firewall profile

- **WHEN** bash runs on a host that cannot enforce the default profile
- **THEN** commands run under the firewall profile with secret-bearing environment scrubbed
- **AND** the result honestly reports the firewall profile so the model can see the limits

### Requirement: Web tool surface

The system SHALL provide `web_search` and `web_fetch` tools that fetch and search external URLs, validating URL schemes against an allowlist (`http`/`https` only), enforcing output caps (search result entries and fetched bytes per scrape), and returning truncated and time-limited results. These tools SHALL be disabled by default and become available only when enabled by policy, and the call-consuming turns SHALL NOT resume until the network call completes. Fetching SHALL honor robots.txt when reasonable and SHALL NOT follow redirects past a bounded hop count.

#### Scenario: Network tools require explicit enablement

- **WHEN** `web_fetch` is invoked while the network capability is disabled
- **THEN** the tool returns a structured "network disabled" result and no network request is made

#### Scenario: Fetch output is capped

- **WHEN** a fetched page exceeds the output cap
- **THEN** the result is truncated and marked, and the agent can see that truncation

### Requirement: Context management

The system SHALL bound what enters the model context per turn: tool results SHALL be rewritten to a concise first-portion-visible form beyond which they are elided, and the assembly budget SHALL cap the number of messages and estimated token count sent per model call via `max_tokens`. The system SHALL never silently drop a `tool_result` the executed tool produced; it SHALL always feed back a result carrying at least a truncation marker. Rewriting SHALL be deterministic and described in the tool contract description so the model can request more detail.

#### Scenario: Large tool output is replaced, not dropped

- **WHEN** a tool returns more than the rewrite cap
- **THEN** the stored transcript holds a placeholder of the first `~8k` characters with a marker, and the version forwarded to the model is deterministic
- **AND** no executed tool result is silently absent from the transcript

#### Scenario: Message budget ends the turn cleanly

- **WHEN** the assembled message count exceeds `max_messages` for the session
- **THEN** the turn ends as a budget-exhausted finish (never an error), keeping the session reusable

### Requirement: Concurrent tool execution

The system SHALL execute tool calls from one model response concurrently when consecutive read-only calls appear in response order, bounded by a concurrency cap, with the transcript event order deterministic (start events in response order). Write tools and unknown tools SHALL NOT run concurrently with anything (they serialize against their neighbors). Tool-result messages SHALL be appended in response order regardless of completion order.

#### Scenario: Read-only reads run in parallel, events in order

- **WHEN** a single response contains multiple read-only tool calls
- **THEN** they execute concurrently under the cap and their start/end events appear in the transcript in response order

#### Scenario: Writes serialize against neighboring read-only calls

- **WHEN** a response mixes read-only and write tools
- **THEN** each write executes only after every read-only call that precedes it in response order has finished, and before any read-only call that follows it starts
- **AND** start events and tool results still appear in response order

### Requirement: Multimodal and language tool capability declaration

The system SHALL declare capability gates truthfully: `read_image` SHALL be presented to the model only when the configured LLM announces image support, and the `lsp` tool's `file_system` field SHALL be reported only when the language server is reachable. The `lsp` tool SHALL be declared but return unavailable (not error) when the server is not running.

#### Scenario: read_image hidden on text-only models

- **WHEN** the configured model does not announce image support
- **THEN** `read_image` is absent from the tool schema and the model cannot call it

#### Scenario: lsp file_system honest when server absent

- **WHEN** the language server is not reachable for a session
- **THEN** the `lsp` tool returns unavailable, and its `file_system` field is not reported as if present

### Requirement: Approval-first webui surface and event vocabulary

The system SHALL support the webui as the first approval answerer: it SHALL emit `PermissionRequest` events for policy-gated calls and SHALL define the decision vocabulary that drives the webui review card as the approval seam (permission decision requests) — the `PermissionRequest` event plus a stable `permission_decision` result outcome. The kernel SHALL NOT render UI itself.

#### Scenario: The approval decision is a distinct event outcome

- **WHEN** a gated call is answered through the webui seam
- **THEN** the kernel ends the permission flow with a stable `permission_decision` outcome, distinct from a normal tool finish

### Requirement: Long-running background work isolation

The system SHALL execute long-running background work (network fetches, multipart uploads) in a dedicated long-running-worktime pool, NOT in the agent-loop execution pool, so agent-loop progress is never blocked by slow background work.

#### Scenario: Slow network task does not stall the loop

- **WHEN** a long network call runs while the agent loop needs to continue
- **THEN** the loop continues promptly and the network work completes in the background pool without blocking the loop pool

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