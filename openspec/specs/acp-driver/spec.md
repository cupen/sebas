# acp-driver Specification

## Purpose
Owns the lifecycle of one Claude Code subprocess per sebas session: spawn, resume, streaming event pump, interrupt-and-heal, hang detection with escalating kill, and a guaranteed single terminal event on death. Provides a stable `AcpCommand` / `AcpEvent` vocabulary to the router so the engine underneath (currently `cc-agent-sdk` over stream-json + control protocol) can be replaced without changing the router contract.

## Requirements

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

### Requirement: ACP session resume through session/load

The generic ACP driver SHALL implement resume for native ACP agents: when a session starts with a load request, the driver SHALL issue the ACP `session/load` request for the persisted session id and continue that conversation. When the agent declines or cannot load the session (unknown id, or the agent does not advertise load capability), the driver SHALL fall back to a fresh session with a new id and report `resumed = false`. A fallback SHALL NOT surface a raw protocol error to the caller.

#### Scenario: ACP resume continues the loaded conversation

- **WHEN** a native ACP agent is spawned in load mode with a persisted session id and the agent supports `session/load`
- **THEN** the driver issues a load for that id, the agent streams the resumed conversation in later prompts, and the spawn reports `resumed = true` with the same routing id

#### Scenario: ACP resume falls back to fresh when the session is gone

- **WHEN** the ACP agent responds to `session/load` with an error (conversation not found)
- **THEN** the driver starts a fresh session with a newly minted id
- **AND** the spawn reports `resumed = false` so the caller can tell the user the old conversation is gone

#### Scenario: Resume is rejected, not faked, for agents without load support

- **WHEN** the driver is asked to load a session but the ACP agent does not support load
- **THEN** resume is handled honestly: the fresh-session fallback applies (never a fabricated successful resume)
- **AND** the caller is informed via `resumed = false`

### Requirement: ACP resume loads by the real ACP session id

The generic ACP driver SHALL accept, in a load request, an explicit ACP session id that differs from the routing session id: when the caller provides the agent's real session id, the driver SHALL issue `session/load` with that id. The driver SHALL report the agent's real ACP session id (from `session/new` or the loaded conversation) back to the caller so the mapping can be persisted. When the load is rejected (unknown id or no load capability), the existing fresh-session fallback with `resumed = false` SHALL apply.

#### Scenario: Load uses the provided ACP session id

- **WHEN** a resume caller supplies an ACP session id distinct from the routing id
- **THEN** the driver issues `session/load` with the provided ACP id
- **AND** on success the routing id is unchanged and `resumed` is `true`

#### Scenario: Fresh spawn reports the agent session id

- **WHEN** a fresh ACP session is created
- **THEN** the spawn outcome carries the agent's real ACP session id (from `session/new`)
- **AND** the routing id maps to it for future resumes

#### Scenario: Rejected real-id load falls back to fresh

- **WHEN** the agent rejects `session/load` for the provided ACP session id
- **THEN** a fresh session starts with a new routing id
- **AND** `resumed` is `false`

### Requirement: ACP config options surface on session establishment

The generic ACP driver SHALL parse the `configOptions` from a `session/new` or `session/load` response and expose the model option (id "model", its select values, and the current value) to the caller as part of the spawn outcome. The driver SHALL NOT hardcode or filter model lists; the agent's response is the source of truth.

#### Scenario: Spawn outcome carries the model option

- **WHEN** an ACP session is established and the agent returns a `model` config option
- **THEN** the spawn outcome carries the model select values and current value
- **AND** a session without a model option reports no model surface

### Requirement: ACP model switch command

The driver SHALL accept a session command to set the model and translate it to the standard ACP `session/set_config_option` (`configId = "model"`). A rejected value SHALL surface an explicit error; an agent without the method or option SHALL report the same explicit error.

#### Scenario: SetModel issues set_config_option

- **WHEN** a `SetModel { model_id }` command targets an ACP session
- **THEN** the driver sends `session/set_config_option` with the model value
- **AND** reports success or an explicit model-rejected error

#### Scenario: Unsupported agent reports explicit error

- **WHEN** the agent lacks `set_config_option`/the model option
- **THEN** the driver returns an explicit "model not supported" error instead of failing silently
