## MODIFIED Requirements

### Requirement: Composer toolbar composition

The composer toolbar SHALL place the focused session's read-only agent identity (with the lock affordance) at the bottom-left, and the model chip and submit control at the bottom-right. The bottom-left tool group SHALL carry the session's permission-mode switch, rendered as a compact select: its resting width SHALL fit the selected label (max-content behaviour) within a small cap (about 110px), and its option labels SHALL be Title Case (Ask/Edit/Allow/Auto) — the wire vocabulary stays lowercase. The mode switch SHALL always present a definite value: every session's `desired_mode` SHALL be one of the four control-plane words and SHALL NOT be nullable on the wire or in memory — the `Option<String>` shape is dropped entirely. **The control-plane default is `ask`** — the domain's `SessionMode::#[default]` (every gated action pauses for operator approval). A session created without an explicit mode choice SHALL be stored with `desired_mode = "ask"`, the creation dialog SHALL preselect `ask`, and any pre-existing database row whose stored value is null SHALL be rewritten to `"ask"` by a one-shot startup migration before it is ever projected. There SHALL be no read-path fallback that interprets null as ask — the migration is the single point where null goes away, and every projection afterwards reads the migrated value verbatim. The composer SHALL NOT render an empty or placeholder mode state. The toolbar's left and right tool groups SHALL align on a shared baseline (a stable grid or equivalent), so controls of different intrinsic heights line up horizontally instead of floating apart. The composer SHALL NOT render a settings entry — the app shell owns settings — and SHALL NOT render any creation control; the toolbar mode switch is the mid-session surface, and the creation dialog owns the creation-time choice.

Each of the four control-plane words SHALL map to a definite executor behaviour on every supported agent, or produce an explicit typed report that the executing body cannot honour it:

- **claude**: `ask` maps to the driver's Default permission mode (every gated action asks), `edit` maps to AcceptEdits, `allow` and `auto` map to BypassPermissions; the mapping SHALL be applied both at spawn time (via the driver's argv choice) and mid-session (via the driver's runtime SetMode command) so switching takes real effect.
- **generic ACP agents**: the ACP protocol carries no permission-mode vocabulary; a mode switch SHALL surface an explicit typed notice that the mode cannot be applied to this body and the session's previous mode SHALL remain in effect. The system SHALL NOT pretend the switch succeeded.
- **native (agent-*) sessions**: mode switching SHALL surface an explicit typed unavailability report; the system SHALL NOT pretend the switch succeeded.

#### Scenario: toolbar places identity left, actions right

- **WHEN** a session is focused and the composer renders
- **THEN** the locked agent identity appears at the bottom-left and the model chip and submit control appear at the bottom-right

#### Scenario: mode select is compact and titled

- **WHEN** the operator inspects the composer's mode switch
- **THEN** the select hugs its label within the small width cap, the options read Ask/Edit/Allow/Auto, and switching still sends the lowercase wire value

#### Scenario: unset mode is stored and rendered as Ask

- **WHEN** the operator creates a session without explicitly choosing a mode
- **THEN** the session is stored with `desired_mode = "ask"`, the composer presents Ask as the selected mode, and any subsequent read (list, detail, composer binding) returns `"ask"`

#### Scenario: legacy null mode is migrated to ask at startup

- **WHEN** the binary starts against a database whose session rows contain a null `desired_mode`
- **THEN** a startup migration rewrites those rows to `"ask"` before any projection is served, and every subsequent read (list, detail, composer binding) observes `"ask"` — no read-path fallback for null remains in the code

#### Scenario: mode switch on an agent that cannot honour it

- **WHEN** the operator switches the mode on a generic ACP or native session
- **THEN** the system surfaces a typed notice that the executing body cannot apply the new mode, and the session's previous mode remains in effect

#### Scenario: toolbar rows share a baseline

- **WHEN** the toolbar renders controls of different intrinsic heights (select, chip, icon button)
- **THEN** the groups align on one horizontal baseline with no control sitting visibly higher or lower than its neighbours

#### Scenario: no mode control in the composer

- **WHEN** a session is focused and the operator inspects the composer toolbar
- **THEN** the permission-mode switch renders in the toolbar's left tool group as a compact select（本场景随 spec 漂移修正改写：mode 切换已自会话头迁入 composer 底沿左端，本 delta 以代码现状为基线——创建期选择仍归创建对话框）

#### Scenario: no settings entry in the composer

- **WHEN** the operator inspects the composer toolbar
- **THEN** no settings control is rendered; settings remain reachable from the app shell's sidebar entry

### Requirement: Submit control reflects submission and turn state

The composer's submit control SHALL reflect the state of the input and the focused session's turn:

- empty input and no in-flight work: the control SHALL render disabled;
- non-empty input and no in-flight work: the control SHALL render as the enabled send affordance;
- a submission POST in flight: the control SHALL render an in-progress indication instead of the send affordance;
- the focused session has a turn in flight and the input is empty: the control SHALL render as a stop affordance, and activating it SHALL request cancellation of the session's in-flight turn;
- the focused session has a turn in flight and the input is non-empty: the control SHALL render a queued affordance, and submitting SHALL enqueue the submission via the existing turn queue without dropping or replacing pending entries;
- the focused session's child is starting (spawn requested or in flight, no live turn yet) with the input non-empty: the control SHALL render a starting affordance that is visually distinct from the queued affordance — the submission is staged for the starting child, and the workbench SHALL NOT present it as queued behind a running turn;
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

#### Scenario: starting child stages rather than queues

- **WHEN** the focused session's child is starting and the operator submits a message
- **THEN** the control renders the starting affordance, distinct from the queued affordance, and the message is staged for the child that is starting

#### Scenario: cancel does not drop pending submissions

- **WHEN** the operator cancels the in-flight turn while pending submissions are queued
- **THEN** the in-flight turn is interrupted and the pending submissions remain queued

## ADDED Requirements

### Requirement: Session status lives in one place

Session-lived status SHALL be presented exactly once, on the rail: each session row SHALL carry a compact coloured dot keyed by the session's status slug (starting, queued, working, waiting, done, failed, dormant), using the existing `--sebas-status-*` token set. The same session's status SHALL NOT be re-expressed in the conversation stage — the session-head card SHALL present identity and action affordances (chat, agent, model, mode, actions) and SHALL NOT render a status badge (no `<sebas-status-badge>` with the session's slug/label/glyph), SHALL NOT render a status-coloured border, banner, or equivalent treatment. The project header SHALL NOT render a session-count badge, an active/idle pill, or a second copy of the focused session's status badge; a focused-session link MAY keep the session's stable chat identity as the anchor text, but SHALL NOT re-express the session's status slug. Message-level queueing SHALL remain an attribute of the message surface (the pending stack above the composer) and SHALL NOT be projected onto any session-level chrome — no header, banner, or rail text SHALL label a session as "queued" because of queued messages. The `<sebas-status-badge>` component itself MAY remain for other views that still use it; this requirement governs the workbench surface only.

#### Scenario: rail dot reflects each session's status slug

- **WHEN** the rail lists sessions in mixed states (e.g. one working, one queued, one failed)
- **THEN** each row carries the coloured dot matching its own slug, and that dot is the only per-row expression of status

#### Scenario: session-head card renders no session-status treatment

- **WHEN** the operator focuses a session whose slug is `queued`, `working`, `waiting`, or any other
- **THEN** the session-head card renders identity (chat / agent / model / mode / actions) without a `<sebas-status-badge>` for the session status, without a status-keyed border, banner, or color

#### Scenario: project header has no session-count or status-badge copies

- **WHEN** the operator views the workbench with sessions in any state
- **THEN** the project header renders the project name, execution node chip, and branch pill, and SHALL NOT render a `X sessions` count, an `active`/`idle` pill, or a `<sebas-status-badge>` inside the focused-session link

#### Scenario: queued messages do not leak into session chrome

- **WHEN** a session has queued messages while its own slug is `working`
- **THEN** the queued count appears only in the pending stack above the composer; nothing on the session row, session-head card, or project header marks the session itself as queued

### Requirement: Compact island spacing

The workbench's floating surfaces SHALL be separated by the compact spacing token (8px): the gap between the project rail and the main area, and the seam between the conversation stage and the composer, SHALL each present as a narrow breathing space rather than a wide band; the stage column and the composer column SHALL share identical horizontal padding so their content edges line up. Reducing the gaps SHALL NOT eliminate the draggable boundaries — both resize affordances SHALL remain reachable.

#### Scenario: rail and main area sit close together

- **WHEN** the operator views the workbench at desktop width
- **THEN** the space between the rail island and the main area matches the compact spacing token, with the divider still hoverable for resizing

#### Scenario: stage and composer edges align

- **WHEN** the operator compares the stage island's and the composer island's horizontal content edges
- **THEN** both columns share the same inset, and the vertical breathing space between them matches the compact spacing token while remaining draggable
