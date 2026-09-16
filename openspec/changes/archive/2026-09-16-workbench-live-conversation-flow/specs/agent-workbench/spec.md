## MODIFIED Requirements

### Requirement: New session without prompt

The workbench SHALL support creating a 0-turn placeholder session without requiring a prompt. The placeholder SHALL appear in the session list immediately and SHALL be activated. An ACP child SHALL NOT block creation: focusing a placeholder session that has no live child SHALL start the child in the background — resuming the recorded conversation when the session mapping allows it and reporting honestly when it does not — and the first message SHALL also start the child if focus never did (for example the operator submits from the rail preview without switching). Clicking it SHALL open a creation dialog — the ONLY place an agent can be chosen — which SHALL require an explicit agent choice drawn from `/api/agents` (no implicit or "null" agent), SHALL preselect the target project's remembered default agent when one exists, SHALL carry an optional permission-mode choice (`ask | edit | allow | auto`, defaulting to "agent default" which omits the `mode` field on the wire), and MAY carry an optional two-level model choice (provider, then model) drawn from the Settings catalog, preselected per the configured default provider and model. Confirming the dialog SHALL create and activate the placeholder; cancelling SHALL create nothing. A failed background start SHALL NOT kill the placeholder: it stays in the list and the failure is surfaced where it happened.

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

- **WHEN** the operator sends a message into a zero-turn placeholder session whose child was never started (or has not finished starting)
- **THEN** the system spawns the ACP child and the session transitions to working

#### Scenario: dialog carries the permission-mode choice

- **WHEN** the creation dialog is open and the operator leaves the mode dropdown on its default
- **THEN** confirming creates the session without a `mode` field on the wire; choosing `edit` creates it with `mode=edit`

#### Scenario: focusing the placeholder starts the child with resume

- **WHEN** the operator focuses a placeholder session that has no live child, and the session carries a persisted conversation mapping
- **THEN** the child starts in the background and continues the recorded conversation, and the session becomes usable without the operator sending a prompt first

#### Scenario: failed background start keeps the placeholder

- **WHEN** the background start of a placeholder's child fails
- **THEN** the session remains in the list as a placeholder and the failure is stated, not silently swallowed

### Requirement: Session archive

The rail session row's overflow menu SHALL be the ONLY operator-facing archive entry: archiving moves the session to the History group and carries the close semantics (the child is killed if active and staged or queued submissions are discarded, which the confirm dialog warns about with the pending count). The focused session's header SHALL render no action buttons and no navigation link — no "All sessions", no Archive, no Close, and no mode switcher. An archived session SHALL be read-only — the operator cannot send messages into it, cannot close it, and cannot switch to it as the active session. An archived session SHALL be restorable to its original project by clicking it in the History group.

#### Scenario: archive a session

- **WHEN** the operator picks archive in a rail session row's overflow menu and confirms
- **THEN** the session is moved to the History group, marked as read-only, and any pending submissions are reported as discarded in the confirm dialog before the action

#### Scenario: restore archived session

- **WHEN** the operator clicks an archived session in the History group
- **THEN** the session is restored to its original project, becomes writable, and is activated

#### Scenario: focused session header offers no actions

- **WHEN** a session is focused and the operator looks at the session header
- **THEN** the header shows display information only — no action buttons, no mode switcher, and no "All sessions" link

### Requirement: Model selector offers the backend catalog before any session

The creation dialog's model selector SHALL offer the catalog the operator configured in Settings — every configured provider's model list, presented as two levels (provider, then model) — so the operator can pick a model for the first turn of a new session. There is no configured default provider/model feeding this preselection: the dialog SHALL preselect, in order, the operator's last-used (provider, model) pair — remembered globally in the browser and written only by a creation-dialog confirmation — when that pair still exists in the catalog; otherwise the catalog's first pair. When the catalog is empty or unavailable, the dialog SHALL NOT offer an empty or fabricated list: it SHALL present an explicit indication that no models are configured and direct the operator to Settings → Models to add one. Catalog changes SHALL be reflected in the dialog without requiring a restart; a remembered pair that has vanished from the catalog SHALL NOT be preselected.

The workbench composer SHALL present the focused session's model selection as a single chip at the composer's bottom-right, offering that session's `available_models`, because a mid-session switch is valid only if the session's execution body accepts the chosen model. The chip SHALL NOT derive its options from the catalog or from another session's `available_models`, and switching a session's model SHALL NOT write the last-used pair remembered for creation. While the focused session's child is starting (the eager start of a placeholder or a re-opened session), the chip SHALL state that startup is in progress rather than claiming no models exist. When the focused session's child has finished starting and exposes no models, the chip SHALL state that honestly rather than rendering an empty menu.

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
- **THEN** the model area states that no models are configured and directs the operator to Settings → Models, rather than rendering an empty selector

#### Scenario: chip without session models is stated honestly

- **WHEN** the focused session exposes no `available_models` after its child has finished starting
- **THEN** the chip presents an explicit unavailability indication rather than an empty menu

#### Scenario: chip states startup in progress

- **WHEN** the focused session's child is starting and has not reported models yet
- **THEN** the chip states that the agent is starting rather than showing "no models available"

## ADDED Requirements

### Requirement: Workbench chrome density and alignment

The workbench layout SHALL keep the rail, conversation stage, and composer visually adjacent: the draggable boundary between rail and main area SHALL be slim (at most 8px rendered divider), the conversation stage and the composer SHALL share the same horizontal inset so their left and right edges align, and the vertical gap between the stage and the composer SHALL be minimal. On viewports at least 1440px wide, the rail SHALL default to 280px with a drag ceiling of 520px, sized so a session row title keeps at least 12 CJK characters visible at the standard title size.

#### Scenario: edges align between stage and composer

- **WHEN** the workbench renders with a focused session
- **THEN** the conversation stage and the composer share the same left and right edges

#### Scenario: slim divider

- **WHEN** the rail is resized
- **THEN** the visible divider between rail and main area stays within 8px

#### Scenario: rail shows twelve CJK characters

- **WHEN** a rail session row has a 12-character Chinese title and the rail is at its default width on a viewport of at least 1440px
- **THEN** the title is visible without truncation
