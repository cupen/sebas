## Purpose

Owns the lifecycle of one Claude Code subprocess per sebas session: spawn, resume, streaming event pump, interrupt-and-heal, hang detection with escalating kill, and a guaranteed single terminal event on death. Provides a stable `AcpCommand` / `AcpEvent` vocabulary to the router so the engine underneath (currently `cc-agent-sdk` over stream-json + control protocol) can be replaced without changing the router contract.

## ADDED Requirements

### Requirement: One subprocess per session

The system SHALL spawn exactly one Claude Code child process per sebas session. The sebas routing id SHALL be the same value as the Claude conversation id.

#### Scenario: Fresh spawn mints a new id

- **WHEN** the manager is asked to create a new session
- **THEN** a fresh UUID is minted
- **AND** the child is launched with `--session-id <uuid>` so the Claude conversation id equals the routing id

#### Scenario: Resume reuses the existing id

- **WHEN** the manager is asked to resume a previously persisted session id
- **THEN** the child is launched with `--resume <id>` only
- **AND** `--session-id` is NOT passed (the real CLI rejects that combination)
- **AND** the routing id remains the resumed conversation id

### Requirement: Startup handshake with timeout

The system SHALL complete the SDK initialize handshake within a configurable `startup_timeout`. On timeout or connection failure, the system SHALL abort the spawn and surface an error; the child process SHALL be killed.

#### Scenario: Startup timeout kills the child

- **WHEN** the initialize handshake does not complete within `startup_timeout`
- **THEN** the connect attempt fails with a timeout error that includes the captured child stderr tail
- **AND** the child process is terminated

#### Scenario: Hermetic child environment

- **WHEN** the child is spawned
- **THEN** `setting_sources` is set to an empty list so the child never loads the host user's Claude settings or hooks
- **AND** only the explicitly provided `extra_env` entries are merged on top of the OS environment

### Requirement: Resume rejection falls back to fresh session

The system SHALL detect a resume rejection (Claude reports "No conversation found") and transparently fall back to a fresh session with a new id, rather than surfacing a raw spawn error.

#### Scenario: Rejected resume spawns fresh

- **WHEN** the manager attempts `SessionStart::Load(old_id)` and the child reports "No conversation found"
- **THEN** the connect returns `ResumeRejected`
- **AND** the manager spawns a new session with a freshly minted id
- **AND** `SpawnOutcome.resumed` is `false` so the caller can inform the user the old conversation is gone

#### Scenario: Successful resume keeps the id

- **WHEN** the manager attempts `SessionStart::Load(old_id)` and the resume succeeds
- **THEN** `SpawnOutcome.session_id` equals `old_id`
- **AND** `SpawnOutcome.resumed` is `true`

### Requirement: Streaming event pump

The system SHALL stream agent events to the router as they occur, without waiting for turn completion. The driver SHALL translate SDK messages into the stable `AcpEvent` vocabulary and SHALL track tool `tool_use_id → tool_name` so `tool_result` frames can be surfaced with the correct tool name.

#### Scenario: Events stream during a turn

- **WHEN** the agent is mid-turn producing thinking, text, or tool calls
- **THEN** each increment is emitted as an `AcpEvent` on the session's event channel as it arrives

#### Scenario: Tool results carry the tool name

- **WHEN** a `User(tool_result)` frame arrives carrying only a `tool_use_id`
- **THEN** the driver looks up the previously recorded tool name for that id
- **AND** emits the corresponding `ToolEnd`-style event with the tool name populated

### Requirement: Cancel via interrupt and respawn-with-resume

The system SHALL implement turn cancellation as `interrupt()` followed by a transparent respawn of the child with `resume = session_id`, because the Claude CLI is unusable after an interrupt.

#### Scenario: /cancel heals the session

- **WHEN** a `Cancel` command is issued mid-turn
- **THEN** the current child is interrupted
- **AND** a new child is spawned with `resume` set to the same session id
- **AND** the session routing id is unchanged from the router's perspective
- **AND** subsequent prompts continue the same conversation

### Requirement: Hang detection with escalating kill

The system SHALL detect a hung child while a turn is active and escalate: `interrupt()` up to 3 times, then disconnect (≈SIGTERM), then drop (≈SIGKILL). Hang detection SHALL be suspended while a permission request is parked awaiting user click, and SHALL NOT fire when no turn is active.

#### Scenario: No activity during a turn triggers escalation

- **WHEN** the child produces no message for the configured hang timeout while a turn is active
- **THEN** the driver issues `interrupt()` up to 3 times
- **AND** if the child still does not respond, disconnects the transport
- **AND** if the child persists, drops the client (SIGKILL)

#### Scenario: Permission wait is never a hang

- **WHEN** a PreToolUse permission request is parked awaiting user click
- **THEN** the hang detector is suspended
- **AND** the child is not interrupted regardless of elapsed time

#### Scenario: Idle child is not killed

- **WHEN** no turn is active (the child is waiting for the next prompt)
- **THEN** hang detection does not fire

### Requirement: Single terminal event guarantee

The system SHALL emit exactly one terminal `AcpEvent::Error{terminal: true}` when a session dies without an explicit `kill`, regardless of the cause (crash, EOF, watchdog, connect failure).

#### Scenario: Child exit surfaces one terminal event

- **WHEN** the driver loop exits without an explicit `kill` and without a prior terminal event
- **THEN** exactly one `Error{terminal: true}` event is emitted
- **AND** the session entry is removed from the manager's table

#### Scenario: Explicit kill does not emit terminal event

- **WHEN** `kill(session_id)` is invoked
- **THEN** the cancel sender fires
- **AND** the driver loop exits
- **AND** no synthetic terminal error is emitted

### Requirement: Provider-driven environment injection

The system SHALL merge `extra_env` (e.g. `ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN`, `OPENAI_BASE_URL`, `OPENAI_API_KEY`) into the child process environment at spawn, on top of the OS environment. The same injection SHALL apply to both fresh spawns and resumes.

#### Scenario: Direct mode injects Anthropic env

- **WHEN** the router resolves provider mode to Direct with an Anthropic-protocol provider
- **THEN** the spawn passes `ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN` via `extra_env`
- **AND** the child uses those values instead of any OS-level values

### Requirement: Permission reply routing

The system SHALL route `AcpCommand::PermissionReply` to the parked oneshot for the given `request_id`, bypassing the command channel. A reply for an unknown `request_id` SHALL be logged and dropped without erroring the caller.

#### Scenario: Reply resolves the parked oneshot

- **WHEN** a `PermissionReply { request_id, decision }` arrives and a responder is parked under that id
- **THEN** the oneshot is removed from the pending map
- **AND** the decision is delivered to the driver's hook callback

#### Scenario: Reply for unknown id is dropped

- **WHEN** a `PermissionReply` arrives for a `request_id` with no parked responder
- **THEN** the reply is logged with the set of currently-known request ids
- **AND** the call returns `Ok(())`
