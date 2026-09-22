## MODIFIED Requirements

### Requirement: Unseen-turn seam

The turn stream SHALL mark the boundary between turns the operator has already
seen and those that arrived since, showing how many arrived and over what span.
When a session has unseen turns, opening it SHALL position the stream at that
boundary rather than at the newest turn. The seen-boundary SHALL be per-browser
state and SHALL NOT be recorded server-side. The boundary SHALL fall between two turns and SHALL count turns rather than transcript entries, so one agent turn rendered as a single bubble is never split across the seam.

Turns that arrive while their session is focused and the document is visible
SHALL advance the seen-boundary as they render, sharing the unread cursor
semantics with the `session-unread-badge` capability: content the operator has
watched arrive SHALL NOT be flagged by the seam on a later reopen. This holds
in particular for the first exchange of a freshly created placeholder session
(no scroll position to preserve and no prior boundary). The seam SHALL appear
only for turns that arrived while the session was not focused, or while the
page was hidden, or while the operator was scrolled away from the live edge.

#### Scenario: opening a session with unseen turns

- **WHEN** the operator opens a session that received turns since their last
  visit
- **THEN** the stream opens positioned at the boundary, with the boundary
  marked and the count of turns below it stated

#### Scenario: nothing unseen

- **WHEN** the operator opens a session with no turns since their last visit
- **THEN** no boundary is drawn and the stream opens at the newest turn

#### Scenario: boundary is per-browser

- **WHEN** the operator opens the same session from a different browser
- **THEN** that browser's own seen-boundary applies, and the server holds no
  record of either

#### Scenario: the seam never splits a turn

- **WHEN** the operator opens a session whose unseen turns include one long agent turn composed of streamed text, thinking and tool calls
- **THEN** the boundary is drawn above that whole turn and no part of it appears on the seen side
- **AND** the stated count is the number of turns below the boundary, not the number of transcript entries

#### Scenario: watched arrivals leave no seam

- **WHEN** the operator sends a message into the focused, visible session and the reply completes while they remain at the live edge, then closes and reopens the session
- **THEN** no seam boundary is drawn for those turns and no "~N new since you last viewed" marker appears

#### Scenario: first exchange of a placeholder session leaves no seam

- **WHEN** the operator creates a placeholder session, sends the first message, and watches the spawned child's first reply in the focused view
- **THEN** the anchor is established from the empty-stream state and the reply is treated as seen

### Requirement: Execution-body availability is stated, not discovered

The composer's execution-body selector SHALL reflect, for each execution body,
whether it can serve new sessions in the current process configuration. An
execution body that cannot serve new sessions — for example the native kernel
running without provider credentials — SHALL be presented as unavailable with
its cause stated, and SHALL NOT be selectable such that the operator only
discovers the failure on submission. Availability SHALL be derived from the
session backend's own report of both execution bodies, not from the ACP side
alone.

The cause SHALL be stated in operator-facing language: it SHALL name the
remediation surface (for example "未配置模型凭据 — 到 Settings → Models 配置
provider"), and SHALL NOT expose internal environment variable names or other
implementation identifiers in the default-visible copy; such identifiers MAY
appear in a tooltip or help link only.

#### Scenario: native kernel without credentials shown as unavailable

- **WHEN** the native kernel has no provider credentials and the composer is
  rendered
- **THEN** the `native` option is shown as unavailable with the cause stated,
  and submitting a native spawn is prevented at the composer rather than
  failing at the core

#### Scenario: unavailable cause speaks operator language

- **WHEN** the native kernel is unavailable for lack of provider credentials
- **THEN** the visible cause names the remediation surface and contains no
  environment variable identifier such as `SEBAS_AGENT_PROVIDER_API_KEY`

#### Scenario: both bodies available

- **WHEN** both the ACP bridge and the native kernel can serve new sessions
- **THEN** the selector offers both without degradation notices

#### Scenario: availability recovers without reload

- **WHEN** the cause making an execution body unavailable is resolved while the
  page stays open
- **THEN** the selector offers that body again without the operator reloading

### Requirement: Model selector offers the backend catalog before any session

The creation dialog's model selector SHALL offer the catalog the operator configured in Settings — every configured provider's model list, presented as two levels (provider, then model) — so the operator can pick a model for the first turn of a new session. There is no configured default provider/model feeding this preselection: the dialog SHALL preselect, in order, the operator's last-used (provider, model) pair — remembered globally in the browser and written only by a creation-dialog confirmation — when that pair still exists in the catalog; otherwise the catalog's first pair. When the catalog is empty or unavailable, the dialog SHALL NOT offer an empty or fabricated list: it SHALL present an explicit indication that no provider models are configured and direct the operator to Settings → Models to add one. That indication SHALL also state that proceeding without a selection runs the session on the agent's own default model, so the indication does not imply that session creation is blocked when it is not. Catalog changes SHALL be reflected in the dialog without requiring a restart; a remembered pair that has vanished from the catalog SHALL NOT be preselected.

The workbench composer SHALL present the focused session's model selection as a single chip at the composer's bottom-right, offering that session's `available_models`, because a mid-session switch is valid only if the session's execution body accepts the chosen model. The chip SHALL NOT derive its options from the catalog or from another session's `available_models`, and switching a session's model SHALL NOT write the last-used pair remembered for creation. While the focused session's child is starting (the eager start of a placeholder or a re-opened session), the chip SHALL state that startup is in progress rather than claiming no models exist. When the focused session's child has finished starting and exposes no models, the chip SHALL state that honestly rather than rendering an empty menu. The chip SHALL note that its options come from the session's execution body, so an empty provider catalog and a populated chip are not read as a contradiction.

#### Scenario: selector populated before any session

- **WHEN** the operator opens the creation dialog with a non-empty catalog and no last-used pair remembered
- **THEN** the dialog's model selector offers the catalog's models with the catalog's first pair preselected

#### Scenario: provider and model are chosen in two levels

- **WHEN** the creation dialog is open with two providers configured in Settings
- **THEN** the selector first offers the providers, and choosing one offers that provider's models

#### Scenario: last-used pair wins the preselection

- **WHEN** the operator previously confirmed a creation with `openai / gpt-5` and opens the creation dialog again with `gpt-5` still present in the catalog
- **THEN** the selector preselects provider `openai` and model `gpt-5`, not the catalog's first pair

#### Scenario: vanished last-used pair falls back to the first pair

- **WHEN** the remembered last-used pair's model (or provider) no longer exists in the catalog
- **THEN** the selector preselects the catalog's first pair instead, without surfacing the stale pair as an option

#### Scenario: creating writes the memory, session switches do not

- **WHEN** the operator confirms a creation with a chosen (provider, model) pair, or separately switches a focused session's model via the composer chip
- **THEN** only the creation confirmation updates the remembered last-used pair; the session-level switch leaves it untouched

#### Scenario: existing sessions keep session-sourced options

- **WHEN** a session exposes `available_models` and the operator opens the composer's model chip
- **THEN** the chip offers that session's options in a two-level menu grouped by provider, with the current model marked

#### Scenario: no catalog and no session models is stated honestly

- **WHEN** no provider catalog exists and the focused session exposes no models
- **THEN** the creation dialog and the composer chip each present an explicit unavailability indication rather than an empty list

#### Scenario: empty catalog directs the operator to configure one

- **WHEN** the creation dialog opens with an empty or unavailable catalog
- **THEN** the model area states that no provider models are configured and directs the operator to Settings → Models, rather than rendering an empty selector

#### Scenario: empty catalog does not imply blocked creation

- **WHEN** the creation dialog opens with an empty provider catalog while the chosen execution body still serves sessions with its own default model
- **THEN** the indication states that creating now runs on the agent's default model, and the create action remains enabled

#### Scenario: chip without session models is stated honestly

- **WHEN** the focused session exposes no `available_models` after its child has finished starting
- **THEN** the chip presents an explicit unavailability indication rather than an empty menu

#### Scenario: chip states startup in progress

- **WHEN** the focused session's child is starting and has not reported models yet
- **THEN** the chip states that the agent is starting rather than showing "no models available"

#### Scenario: chip names its source

- **WHEN** the operator opens the composer's model chip while no provider catalog is configured
- **THEN** the chip presents the session's own model options and notes they come from the session's execution body

## ADDED Requirements

### Requirement: Session creation failure is surfaced, not silent

When a creation attempt from the new-session dialog does not produce a session
— whether the submit is not dispatched (for example the owning project is not
currently selected) or the backend rejects it — the dialog SHALL present an
inline error naming the cause, and SHALL NOT close itself as if the creation
had succeeded. A creation that succeeds SHALL land a visible session row
under the owning project without requiring the operator to select the project
first.

#### Scenario: creation with no dispatched request is not silent

- **WHEN** the operator submits the new-session dialog in a state where no
  creation request is dispatched (for example the owning project is registered
  but not selected)
- **THEN** either the request is dispatched normally, or the dialog presents
  an inline error — in no case does the dialog close with no session created
  and no feedback

#### Scenario: failed backend creation stays in the dialog

- **WHEN** the backend rejects a creation request
- **THEN** the dialog stays open with the rejection reason shown inline, and
  the operator's inputs are preserved for retry
