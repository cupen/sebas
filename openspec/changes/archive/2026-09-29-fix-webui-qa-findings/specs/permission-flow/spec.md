## ADDED Requirements

### Requirement: 模式切换在转写留痕

A confirmed permission-mode switch on a session SHALL produce a persisted,
mode-specific transcript entry (the existing `permission_mode_result` entry
kind) recording the requested mode and the effective outcome. The entry
SHALL be written by the session backend at the point the switch is applied —
not synthesized client-side — so it survives page reloads and appears in
every connected client's transcript for that session. A switch that fails
SHALL surface its failure through the normal error surfaces and SHALL NOT
write a success contract entry. The transcript entry kind SHALL remain
renderable by the WebUI transcript view as a first-class entry (not folded
into generic markdown).

#### Scenario: 切换后条目即时落转写

- **WHEN** the operator switches a started session's mode via the composer
  menu and the switch is applied
- **THEN** a `permission_mode_result` entry appears in the session transcript
  naming the effective mode, without a page reload

#### Scenario: 条目跨刷新存活

- **WHEN** the operator reloads the page after a mode switch
- **THEN** the transcript still contains the mode-switch contract entry for
  that switch

#### Scenario: 切换失败不写成功条目

- **WHEN** a mode switch request is rejected (unknown mode, session not
  started)
- **THEN** no success contract entry is written; the failure is presented
  through the existing typed-rejection surface

### Requirement: 审批决策结果可见

After a parked permission request is decided (allow once, allow for
session, deny, or escalate), the outcome SHALL be observable in the UI
without manual unfolding: the tool entry produced by the approved or denied
call SHALL present its execution outcome (success payload or typed denial)
at the entry's top level, and a denied call SHALL be visually
distinguishable from a successful one. Deep folding (a process group fold
inside another fold) SHALL NOT be the only way to reach a tool result's
outcome text.

#### Scenario: 放行后结果可见

- **WHEN** the operator allows a gated tool call and the tool executes
- **THEN** the corresponding tool entry shows its success outcome at the top
  level without requiring two levels of manual expansion

#### Scenario: 拒绝结果可辨识

- **WHEN** the operator denies a gated tool call
- **THEN** the tool entry renders the denial as an error-class outcome that
  is distinguishable from a success at a glance
