# agent-workbench Delta

## MODIFIED Requirements

### Requirement: Model selector offers the backend catalog before any session

The creation dialog's model selector SHALL offer the catalog the operator configured in Settings — every configured provider's model list, presented as two levels (provider, then model) — so the operator can pick a model for the first turn of a new session. There is no configured default provider/model feeding this preselection: the dialog SHALL preselect, in order, the operator's last-used (provider, model) pair — remembered globally in the browser and written only by a creation-dialog confirmation — when that pair still exists in the catalog; otherwise the catalog's first pair. When the catalog is empty or unavailable, the dialog SHALL NOT offer an empty or fabricated list: it SHALL present an explicit indication that no models are configured and direct the operator to Settings → Models to add one. Catalog changes SHALL be reflected in the dialog without requiring a restart; a remembered pair that has vanished from the catalog SHALL NOT be preselected.

The workbench composer SHALL present the focused session's model selection as a single chip at the composer's bottom-right, offering that session's `available_models`, because a mid-session switch is valid only if the session's execution body accepts the chosen model. The chip SHALL NOT derive its options from the catalog or from another session's `available_models`, and switching a session's model SHALL NOT write the last-used pair remembered for creation. When the focused session exposes no models, the chip SHALL state that honestly rather than rendering an empty menu.

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

- **WHEN** the focused session exposes no `available_models`
- **THEN** the chip presents an explicit unavailability indication rather than an empty menu
