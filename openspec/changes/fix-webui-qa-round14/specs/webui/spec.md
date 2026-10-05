## ADDED Requirements

### Requirement: Session creation rejections are surfaced

When `POST /api/sessions` (or the follow-up focus/navigation it triggers) is
rejected with a typed rejection, the webui SHALL present that rejection's
operator-facing message at the point of action. The presentation SHALL reuse
the backend's message text (e.g. 「会话数已达上限 32」 for a capacity
rejection) so the operator learns the actual cause, and the UI SHALL NOT
navigate to a session URL that was never created (no phantom deep link).

#### Scenario: capacity rejection is shown at the point of action

- **WHEN** the dispatch engine is at capacity and the operator submits the
  creation dialog
- **THEN** the webui presents the typed rejection message (会话数已达上限 N)
  as a notice on the creation surface, and the browser stays on the workbench
  instead of navigating to a non-existent session

#### Scenario: no phantom session URL

- **WHEN** a session creation request fails for any typed reason
- **THEN** no `/sessions/<key>` URL for the failed creation is pushed into the
  history, and revisiting the workbench shows no ghost entry for it

### Requirement: Model aliases are first-class in model selection surfaces

The composer's model menu and the creation dialog's model selector SHALL
include model alias short names from the alias store alongside provider
catalog models, so the operator can pick a model by its alias without free
text entry. Alias entries SHALL be visually distinguishable from provider
catalog entries (source marker), and selecting one SHALL submit the alias
name as the model value. The alias editor's target-provider dropdown SHALL
list providers from the store and the config.toml seed entries; seed-only
entries SHALL be marked and SHALL NOT be selectable as creation targets —
the stated reason is that an alias can only bind a store provider (the state
store's aliases table enforces this by foreign key). When no store provider
exists at all, the dropdown SHALL state why no target is selectable instead
of being silently inert.

#### Scenario: alias appears in the composer model menu

- **WHEN** an alias `my-claude` exists and the operator opens the focused
  session's model menu
- **THEN** `my-claude` is offered as a choice with a source marker, selecting
  it sends `my-claude` as the model value, and the session's model display
  reflects the selection

#### Scenario: alias appears in the creation dialog

- **WHEN** the operator opens the creation dialog's model selector
- **THEN** alias short names are offered alongside provider catalog models

#### Scenario: alias editor lists seed providers as marked, non-target rows

- **WHEN** the operator opens the alias editor's target-provider dropdown and
  a provider exists only as a config.toml seed entry (not a store row)
- **THEN** that provider is listed with a config-seed marker but is disabled
  as a creation target with the stated reason (别名只能绑定 store provider)

#### Scenario: alias editor empty state explained

- **WHEN** no provider exists in store or seed entries and the operator opens
  the alias editor
- **THEN** the target-provider control is disabled with a stated reason
  (no provider configured yet), not silently inert

### Requirement: Sessions overview page honors role visibility

The `/sessions` overview page SHALL consume the same role→visibility mapping
as the workbench rail: entry points whose permission the current role lacks —
session creation, per-session focus/close and other write actions — SHALL be
hidden (or clearly disabled with the reason) for roles without
`sessions.write`, and any action the backend rejects SHALL surface an explicit
permission-denied presentation. Error banners on the page SHALL distinguish
「无权限」（a rejected action） from 「加载失败」（a failed read）.

#### Scenario: viewer sees no write entries on /sessions

- **WHEN** a viewer opens the `/sessions` overview page
- **THEN** the creation form and per-session write action buttons are not
  offered, while the read-only listing renders normally

#### Scenario: rejected action presents permission denial

- **WHEN** a viewer triggers an action that the backend rejects with 403
- **THEN** the webui presents an explicit permission-denied notice naming the
  role restriction, instead of failing silently

#### Scenario: banner semantics distinguish denial from failure

- **WHEN** a write action is rejected by permission versus a read request
  fails
- **THEN** the former shows 无权限 wording and the latter shows 加载失败
  wording; a permission denial SHALL NOT replace the whole listing with a
  load-failure banner
