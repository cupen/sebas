# agent-core delta — sebas-agent (Phase 2: 权限沙箱 / 网络工具 / 上下文管理 / 首个交互面)

## Purpose

Extends the sebas-agent kernel (Phase 1a: per-session turn loops, six-tool set, budget, cancellation) into a usable coding agent: a policy engine that gates destructive and networked operations with explicit human approval and fail-closed defaults (checklist C3), a network tool surface for external research, context management (tool-result rewriting, assembly budgets, concurrent tool execution), optional multimodal and language-server capabilities with honest capability declaration, and the event vocabulary to drive the webui as its first approval answerer.

## ADDED Requirements

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