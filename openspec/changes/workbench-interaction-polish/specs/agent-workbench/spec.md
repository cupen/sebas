# agent-workbench Delta

## MODIFIED Requirements

### Requirement: Composer promises only what the process can do

The composer SHALL deliver every accepted submission to the core over the session channel, in every process configuration, and SHALL NOT accept a message it cannot deliver. When the channel reports the core unreachable, the composer SHALL render disabled with that cause stated, and SHALL become enabled again on reconnection without a manual reload.

The composer SHALL always be in follow-up mode targeting the webui-side focused-session pointer; it SHALL NOT offer session creation — creation lives in the rail's creation dialog. Focusing a session — via the `/api/sessions/{key}/switch` endpoint, the rail creation dialog, or by visiting the session's deep-link page — SHALL update the pointer so that a composer submission follows the focused session. When no session is focused, the workbench SHALL present the no-focus empty state, the composer SHALL render no creation controls, and an explicit hint SHALL direct the operator to the rail's creation entry.

#### Scenario: composer drives in either configuration

- **WHEN** the workbench runs detached or in-process and the core is reachable
- **THEN** the composer is enabled and a sent message reaches the agent in both

#### Scenario: core unreachable disables with a cause

- **WHEN** the session channel reports the core unreachable
- **THEN** the composer is disabled and states that the core is not connected, rather than presenting an enabled control

#### Scenario: no silent discard

- **WHEN** the core is unreachable
- **THEN** no code path accepts a composer submission and reports success

#### Scenario: recovery needs no reload

- **WHEN** the core returns after being unreachable while the page stays open
- **THEN** the composer becomes enabled again without the operator reloading

#### Scenario: focus follows switch and deep-link

- **WHEN** the operator switches to a session (switch endpoint or deep-link page) and then submits the composer
- **THEN** the submission is delivered to that focused session

#### Scenario: no focus means creation mode

- **WHEN** no session is focused (fresh workbench, or the focused session was just closed)
- **THEN** the workbench presents the no-focus empty state, the composer renders no creation controls and no agent selector, and an explicit hint directs the operator to the rail's creation entry instead of offering in-composer creation

#### Scenario: creation mode requires an explicit agent

- **WHEN** the rail creation dialog is open and the operator has not chosen an agent
- **THEN** the dialog's confirm control is disabled until an agent is chosen from the `/api/agents` list — the composer itself renders no agent choice

#### Scenario: no creation chip in the composer

- **WHEN** a session is focused and the operator looks at the composer toolbar
- **THEN** no "new session" control is rendered — a new session can only be started from the rail's creation dialog

### Requirement: New session without prompt

The workbench SHALL support creating a 0-turn placeholder session without requiring a prompt. The placeholder SHALL appear in the session list immediately and SHALL be activated. An ACP child SHALL NOT be spawned until the first message is sent. Each project row SHALL have a dedicated "New session" button (the rail carries no Inbox group — sessions with no project are not created from the workbench). Clicking it SHALL open a creation dialog — the ONLY place an agent can be chosen — which SHALL require an explicit agent choice drawn from `/api/agents` (no implicit or "null" agent), SHALL preselect the target project's remembered default agent when one exists, SHALL carry an optional permission-mode choice (`ask | edit | allow | auto`, defaulting to "agent default" which omits the `mode` field on the wire), and MAY carry an optional two-level model choice (provider, then model) drawn from the Settings catalog, preselected per the configured default provider and model. Confirming the dialog SHALL create and activate the placeholder; cancelling SHALL create nothing.

#### Scenario: dialog requires an explicit agent

- **WHEN** the operator opens the creation dialog and has not chosen an agent
- **THEN** the confirm control is disabled until an agent is chosen from the `/api/agents` list

#### Scenario: create empty session from project

- **WHEN** the operator confirms the creation dialog opened from a project row's `+` button
- **THEN** a new session with zero turns is created bound to that project and the chosen agent, the project is selected, the session is activated, and the composer is ready for the first message

#### Scenario: cancel creates nothing

- **WHEN** the operator cancels the creation dialog
- **THEN** no session is created and no focus change occurs

#### Scenario: first message spawns the child

- **WHEN** the operator sends a message into a zero-turn placeholder session
- **THEN** the system spawns the ACP child and the session transitions to working

#### Scenario: dialog carries the permission-mode choice

- **WHEN** the creation dialog is open and the operator leaves the mode dropdown on its default
- **THEN** confirming creates the session without a `mode` field on the wire; choosing `edit` creates it with `mode=edit`

### Requirement: Model selector offers the backend catalog before any session

The creation dialog's model selector SHALL offer the catalog the operator configured in Settings — every configured provider's model list, presented as two levels (provider, then model) — so the operator can pick a model for the first turn of a new session. The default selection SHALL follow the configured default provider and model. When the catalog is empty or unavailable, the dialog SHALL state its unavailability honestly rather than offering an empty or fabricated list. Changes to the configured catalog or the default SHALL be reflected in the dialog without requiring a restart.

The workbench composer SHALL present the focused session's model selection as a single chip at the composer's bottom-right, offering that session's `available_models`, because a mid-session switch is valid only if the session's execution body accepts the chosen model. The chip SHALL NOT derive its options from the catalog or from another session's `available_models`. When the focused session exposes no models, the chip SHALL state that honestly rather than rendering an empty menu.

#### Scenario: selector populated before any session

- **WHEN** a default provider with a models catalog is configured and the operator opens the creation dialog
- **THEN** the dialog's model selector offers the catalog's models

#### Scenario: provider and model are chosen in two levels

- **WHEN** the creation dialog is open with two providers configured in Settings
- **THEN** the selector first offers the providers, and choosing one offers that provider's models

#### Scenario: existing sessions keep session-sourced options

- **WHEN** a session exposes `available_models` and the operator opens the composer's model chip
- **THEN** the chip offers that session's options in a two-level menu grouped by provider, with the current model marked

#### Scenario: no catalog and no session models is stated honestly

- **WHEN** no provider catalog exists and the focused session exposes no models
- **THEN** the creation dialog and the composer chip each present an explicit unavailability indication rather than an empty list

#### Scenario: chip without session models is stated honestly

- **WHEN** the focused session exposes no `available_models`
- **THEN** the chip presents an explicit unavailability indication rather than an empty menu

### Requirement: Project-level default agent

Each project SHALL remember the agent most recently used to create a session under it. When the operator opens the creation dialog for that project, the agent selector SHALL preselect that remembered agent. The default is stored in the project registry and survives a restart.

#### Scenario: default agent follows last use

- **WHEN** the operator creates a session in project A with `agent = "codex"` and later opens project A's creation dialog
- **THEN** the agent selector preselects `codex`

#### Scenario: different projects remember different agents

- **WHEN** project A was last used with `codex` and project B with `claudecode`
- **THEN** project A's dialog preselects `codex` and project B's dialog preselects `claudecode`

#### Scenario: first visit falls back honestly

- **WHEN** a project has no recorded default agent
- **THEN** the selector preselects the first reachable agent and marks no project default as chosen

## ADDED Requirements

### Requirement: Submit control reflects submission and turn state

The composer's submit control SHALL reflect the state of the input and the focused session's turn:

- empty input and no in-flight work: the control SHALL render disabled;
- non-empty input and no in-flight work: the control SHALL render as the enabled send affordance;
- a submission POST in flight: the control SHALL render an in-progress indication instead of the send affordance;
- the focused session has a turn in flight and the input is empty: the control SHALL render as a stop affordance, and activating it SHALL request cancellation of the session's in-flight turn;
- the focused session has a turn in flight and the input is non-empty: the control SHALL render a queued affordance, and submitting SHALL enqueue the submission via the existing turn queue without dropping or replacing pending entries;
- when the turn ends (or the cancel completes), the control SHALL return to the send affordance.

#### Scenario: empty input is disabled

- **WHEN** the input is empty and the focused session has no turn in flight
- **THEN** the submit control is disabled

#### Scenario: typed input enables send

- **WHEN** the operator types into the composer and the focused session has no turn in flight
- **THEN** the submit control becomes the enabled send affordance

#### Scenario: submission in flight shows progress

- **WHEN** a submission POST is in flight
- **THEN** the control shows an in-progress indication and returns to the send affordance when the request settles

#### Scenario: streaming with empty input offers stop

- **WHEN** the focused session is streaming a turn and the input is empty
- **THEN** the control renders as a stop affordance; activating it sends the cancel request and the control returns to the send affordance once the turn is no longer in flight

#### Scenario: streaming with text offers queueing

- **WHEN** the focused session is streaming a turn and the input is non-empty
- **THEN** the control renders a queued affordance and submitting appends the text to the pending stack

#### Scenario: cancel does not drop pending submissions

- **WHEN** the operator cancels the in-flight turn while pending submissions are queued
- **THEN** the in-flight turn is interrupted and the pending submissions remain queued

### Requirement: Composer toolbar composition

The composer toolbar SHALL place the focused session's read-only agent identity (with the lock affordance) at the bottom-left, and the model chip and submit control at the bottom-right. The composer SHALL NOT render a settings entry — the app shell owns settings — SHALL NOT render any creation control, and SHALL NOT render a permission-mode control: creation-time mode choice lives in the creation dialog, and mid-session mode switching stays in the session header (webui「会话 mode 在 dashboard 可见可切」).

#### Scenario: toolbar places identity left, actions right

- **WHEN** a session is focused and the composer renders
- **THEN** the locked agent identity appears at the bottom-left and the model chip and submit control appear at the bottom-right

#### Scenario: no settings entry in the composer

- **WHEN** the operator inspects the composer toolbar
- **THEN** no settings control is rendered; settings remain reachable from the app shell's sidebar entry

#### Scenario: no mode control in the composer

- **WHEN** a session is focused and the operator inspects the composer toolbar
- **THEN** no permission-mode control is rendered — mode is visible and switchable in the session header, and the creation dialog owns the creation-time choice

### Requirement: Workbench layout is resizable

The workbench SHALL expose two draggable boundaries: between the project rail and the main area (rail width adjustable within a bounded range), and between the conversation stage and the composer (composer height adjustable within a bounded range, the stage taking the remainder). Both dimensions SHALL persist locally in the browser and SHALL be restored on the next load without server round-trips. On narrow screens the workbench SHALL degrade to the fixed layout without draggable boundaries.

#### Scenario: dragging changes the rail width

- **WHEN** the operator drags the rail/main boundary
- **THEN** the rail width follows the pointer within the allowed range and the main area takes the remainder

#### Scenario: dragging changes the composer height

- **WHEN** the operator drags the stage/composer boundary
- **THEN** the composer height follows the pointer within the allowed range and the conversation stage keeps the remainder scrollable

#### Scenario: dimensions survive a reload

- **WHEN** the operator reloads the page after resizing
- **THEN** the rail width and composer height are restored to the persisted values

#### Scenario: narrow screens degrade

- **WHEN** the viewport is narrower than the breakpoint
- **THEN** the boundaries are not draggable and the layout falls back to the fixed arrangement

### Requirement: Workbench regions read as floating islands

The workbench SHALL visually separate its regions by layering, not by hard rules: the app background SHALL use a distinct canvas tone, and the rail, conversation stage, and composer SHALL render as rounded surfaces set off from the canvas by spacing. Draggable boundaries SHALL present no affordance at rest and SHALL reveal a drag affordance on hover. The treatment SHALL respect the design tokens and the reduced-motion preference.

#### Scenario: regions are separated by spacing and tone

- **WHEN** the workbench renders
- **THEN** the rail, conversation stage, and composer read as distinct rounded surfaces against the canvas tone rather than areas divided by hard full-height rules

#### Scenario: drag affordance appears on hover

- **WHEN** the operator hovers a draggable boundary
- **THEN** a drag affordance appears; at rest the boundary shows no persistent affordance
