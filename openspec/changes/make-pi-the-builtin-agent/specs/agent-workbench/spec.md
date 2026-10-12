## MODIFIED Requirements

### Requirement: Session execution over the native agent kernel

The built-in agent slot (wire id `native`, displayed as `PI`) SHALL execute sessions
through the pi coding agent. The slot SHALL always be present and non-deletable, and
SHALL resolve its launch configuration from the pi agent's configured definition (the
`[acp.agents.pi]` seed or the agents-store row) with pi driver defaults as fallback.
Session creation, the turn loop, the tool set, model switching, and cancellation for
this slot SHALL run in the `pi --mode rpc` child process via the pi driver, under pi's
own semantics: no permission requests or approval cards (pi has no permission system),
pi's tool set and usage reporting, and pi's session persistence. The sebas-agent
in-process kernel SHALL remain in the codebase solely for legacy `agent-*` sessions
(resume and archive read paths); newly created `native` sessions SHALL NOT run on the
kernel.

#### Scenario: WebUI session on the native kernel

- **WHEN** a WebUI create-session request carries `agent = "native"`
- **THEN** the session runs in a pi child process (`pi --mode rpc`) instead of the
  in-process kernel, appears in the WebUI snapshot with project_dir honored and zero
  turns until the first prompt

#### Scenario: Feishu session on the native kernel

- **WHEN** feishu is enabled and a feishu inbound message routes to the built-in slot
- **THEN** the session executes on the pi driver and exists in the same shared
  snapshot visible to the WebUI, with its tool traces and completion text readable
  via the WebUI turn-content API

#### Scenario: Default execution body stays ACP

- **WHEN** no execution-body hint is present (feishu or WebUI default)
- **THEN** the session executes on the ACP bridge as today, preserving behavior

#### Scenario: built-in slot follows the pi agent definition

- **WHEN** the operator customizes `[acp.agents.pi]` (args, sessions_dir) and creates
  a session on the built-in slot
- **THEN** the pi child spawns with that definition, without any process restart

#### Scenario: legacy native sessions keep the kernel resume path

- **WHEN** a previously created kernel session (native key) is reopened or restored
  from archive
- **THEN** it routes to the retained kernel path and its recorded history remains
  readable, while newly created native sessions do not reach that path

### Requirement: Execution-body availability is stated, not discovered

The composer's execution-body selector and the creation dialog's agent list SHALL
reflect, for each execution body, whether it can serve new sessions in the current
process configuration. The built-in `native` (PI) slot's availability SHALL be
derived from the pi coding agent's presence (its resolved binary), NOT from provider
credentials: provider credentials configured in sebas SHALL NOT gate this slot,
because pi does not require an interactive login to serve sessions. When pi is absent,
the slot SHALL be presented as unavailable with its cause stated and SHALL NOT be
selectable; the remediation copy SHALL point at the pi installation surface
（Settings → Agent 一键安装 pi）rather than credential configuration. When pi is
present but unauthenticated (`pi auth check` not ready), the slot SHALL remain
selectable with the authentication state stated as an advisory cause. Availability
SHALL be derived from the session backend's own report of the execution bodies, not
from the client alone, and SHALL recover without reload when the blocking cause is
resolved while the page stays open. Other execution bodies keep their existing
availability semantics.

#### Scenario: native kernel without credentials shown as unavailable

- **WHEN** the built-in slot cannot serve new sessions because the pi binary is not
  installed (the slot no longer has a credential gate)
- **THEN** the `native` (PI) option is shown as unavailable with the cause stated,
  and submitting a spawn on it is prevented at the surface rather than failing at the
  core

#### Scenario: unavailable cause speaks operator language

- **WHEN** the built-in slot is unavailable because pi is not installed
- **THEN** the visible cause names the remediation surface
  （Settings → Agent 一键安装 pi）and contains no environment variable identifier such
  as `SEBAS_AGENT_PROVIDER_API_KEY`

#### Scenario: pi present but unauthenticated stays selectable

- **WHEN** pi is installed but `pi auth check` reports not_ready, and no sebas
  provider env applies
- **THEN** the slot remains selectable, with the authentication state stated as an
  advisory cause rather than a blocker

#### Scenario: both bodies available

- **WHEN** both the ACP bridge and the built-in slot can serve new sessions
- **THEN** the selector offers both without degradation notices

#### Scenario: availability recovers without reload

- **WHEN** the blocking cause (pi installation) is resolved while the page stays open
- **THEN** the selector offers the built-in slot again without the operator reloading

### Requirement: Model selection covers the native kernel

The composer SHALL offer model selection for built-in-slot (native/PI) sessions drawn
from the models the pi child advertises (`get_available_models`, `provider/id`
vocabulary — no hardcoded model table), and a model chosen for such a session SHALL
apply to its subsequent turns via the `set_model` round-trip, with the session's
current model visible afterwards. A native session created with no model chosen — or
with a catalog model that does not map into pi's vocabulary — SHALL run on pi's own
default model, and the composer SHALL NOT show a false "selected" state for it. A
`set_model` failure (for example a model whose provider lacks injected credentials)
SHALL surface as a non-terminal error with the session remaining usable. Sessions on
other execution bodies keep their existing model-selection behavior unchanged.

#### Scenario: native session model dropdown is populated

- **WHEN** the operator opens the model dropdown for a built-in-slot session
- **THEN** it lists the models the pi child advertises rather than being empty

#### Scenario: chosen model applies to subsequent turns

- **WHEN** the operator selects a model on a built-in-slot session and sends a message
- **THEN** the turn runs with the selected model (`set_model` → `ModelChanged`), and
  the session's current model is visible afterwards

#### Scenario: unselected falls back to default

- **WHEN** a built-in-slot session is created with no model chosen
- **THEN** its turns run with pi's own default model and the composer shows no false
  "selected" state

#### Scenario: unmapped catalog model fails honestly and session survives

- **WHEN** a model is applied whose provider has no injected credentials in the pi
  child, and pi rejects it
- **THEN** the failure surfaces as a non-terminal error naming the cause, and the
  session remains usable
