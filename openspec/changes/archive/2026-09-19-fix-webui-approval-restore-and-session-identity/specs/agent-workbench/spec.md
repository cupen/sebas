## ADDED Requirements

### Requirement: Focused session drives project context

When the workbench focuses a session (via a rail session click, creation landing, or restore activation), the main region's project context (title and project scope) SHALL follow the focused session's project immediately — without requiring the operator to click the project row. Clicking a project row SHALL continue to select the project independently.

#### Scenario: rail 切换会话后项目标题跟随

- **WHEN** the operator clicks a different session row in the rail while a project context is not selected
- **THEN** the main region's project title shows the focused session's project without any further interaction

#### Scenario: 新建会话落地后项目标题跟随

- **WHEN** a new session is created and focus lands on its placeholder
- **THEN** the main region's project title shows the session's project immediately

#### Scenario: 项目行点击仍独立生效

- **WHEN** the operator clicks a project row itself
- **THEN** the project context switches to that project as before (existing behavior preserved)

### Requirement: Stop reply fully settles the turn

Stopping a turn SHALL be a full settlement: the turn's end SHALL append a visible transcript entry stating the turn was stopped (an error-class entry with a readable cause), and the stop control SHALL disappear once the turn has settled. The settled state SHALL be stable across page reloads — a reloaded page SHALL NOT present a stopped turn as still in flight.

#### Scenario: 停止后 transcript 有停止条目

- **WHEN** the operator stops an in-flight turn
- **THEN** the conversation shows a transcript entry stating the turn was stopped, instead of the user's message silently hanging

#### Scenario: 停止控件随回合结算消失

- **WHEN** a stopped turn has settled (no in-flight phase and no parked approvals)
- **THEN** the composer no longer shows the stop control

#### Scenario: 刷新后不复活在飞状态

- **WHEN** the page is reloaded after a turn was stopped and settled
- **THEN** the session is presented as idle (no stop control), regardless of the pre-reload presentation

### Requirement: Rail expansion state is persistent and predictable

The rail's project expansion (which projects show their session list) SHALL be persisted in the browser and restored on reload. A project whose session is focused SHALL be presented expanded by default when no persisted state exists for it. Expansion SHALL NOT change on its own across refreshes or focus changes.

#### Scenario: 展开状态跨刷新保持

- **WHEN** the operator expands a project's session list and reloads the page
- **THEN** the project's session list is still expanded

#### Scenario: 聚焦会话所在项目缺省展开

- **WHEN** no persisted expansion state exists and a session of project P is focused
- **THEN** project P's session list is shown expanded on load

#### Scenario: 无操作不自行收起

- **WHEN** the page is reloaded with no operator interaction on the rail
- **THEN** the expansion state does not differ from the persisted state

## MODIFIED Requirements

### Requirement: Parked remote approvals surface in the workbench

Permission requests that stayed parked while the control plane was away SHALL be surfaced to the operator on return, grouped so that a session waiting on a decision is distinguishable from one that is working. A session waiting on a parked decision SHALL be presented as waiting, not as running. The workbench SHALL rebuild the permission review surface from the read model when a waiting session is opened or reloaded, merging it with realtime push by `request_id` so no request is rendered twice and no push for an already-decided request resurrects a card.

#### Scenario: returning operator sees what is waiting

- **WHEN** the operator returns to a control plane that was away while requests were parked
- **THEN** every session waiting on a decision is presented as waiting, with its parked requests reachable

#### Scenario: waiting is not reported as working

- **WHEN** a remote session is blocked on an unanswered permission request
- **THEN** the workbench does not present it as actively working

#### Scenario: 刷新后审批面从读模型重建

- **WHEN** the page is reloaded while a session has a parked permission request
- **THEN** the permission review surface (allow / deny actions) is presented again for that request without any new push

#### Scenario: 重建与推送按 request_id 幂等合并

- **WHEN** a request was rebuilt from the read model and a push for the same `request_id` arrives
- **THEN** only one decision surface for that `request_id` is shown
