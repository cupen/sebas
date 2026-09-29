## ADDED Requirements

### Requirement: 项目注册与移除后的焦点一致性

Registering a new project, removing a project, or switching the displayed
project in the rail SHALL reconcile the focused-session view with the rail's
new state: if the currently focused session belongs to a project that was
removed (or is no longer displayed), the workbench SHALL drop to the
session-less empty state for the newly displayed project instead of
continuing to render the stale session's conversation. The main panel SHALL
NOT display a session whose project is absent from the rail while the rail
shows a different project as current. The stale view MAY be restored by
explicit operator action (selecting the session again where still
available), never by inertia.

#### Scenario: 注册新项目后主面板不残留旧会话

- **WHEN** the operator focuses a session of project A, then registers
  project B (the rail switches the displayed project to B)
- **THEN** the workbench shows project B's empty state instead of project
  A's focused conversation

#### Scenario: 移除项目后主面板不残留

- **WHEN** the operator removes the project that owns the currently focused
  session
- **THEN** the workbench drops to the session-less empty state (or the newly
  displayed project's state), never a lingering view of the removed
  project's session

### Requirement: 关键操作回执

Successful state-changing operator actions with no other visible effect
SHALL produce a transient info-level acknowledgement through the
notification layer, so the operator can confirm the action landed. At
minimum this covers: archiving a session, restoring a session, registering
a project, removing a project, renaming a session, and creating a session.
Actions whose success is already visible in place (the rail row updates,
the dialog closes on a visible new row) MAY keep the row update as the sole
receipt; actions whose effect lands elsewhere or is otherwise easy to miss
SHALL get the toast. The acknowledgement SHALL follow the existing
notification-layer levels and stack rules.

#### Scenario: 归档成功有回执

- **WHEN** the operator confirms archiving a session
- **THEN** a transient info acknowledgement naming the action appears (in
  addition to the rail's History group updating)

#### Scenario: 项目移除成功有回执

- **WHEN** a project removal is confirmed and applied
- **THEN** a transient info acknowledgement appears; a rejected removal
  surfaces its typed failure instead

### Requirement: About 分区的 toolchain 探测

The About section SHALL attempt to detect the Rust toolchain version and
display the result. When detection fails or the toolchain is absent, the
field SHALL state an explicit unavailable reason (for example「未安装」or
「探测失败」) instead of a bare「未知」placeholder, so the operator can
distinguish「没装」from「没探测到」. The remainder of the About fields
(version, uptime, default agent, router listen, provider count) keep their
current behavior; richer build segments remain owned by the
`add-about-build-info` change.

#### Scenario: toolchain 存在时显示版本

- **WHEN** the runtime has a detectable Rust toolchain and the operator
  opens About
- **THEN** the toolchain field shows the detected version

#### Scenario: toolchain 缺失时显示明确原因

- **WHEN** detection fails or no toolchain is installed
- **THEN** the field names the cause explicitly instead of showing「未知」

### Requirement: Models 页 provider 来源可解释

The Settings→Models page SHALL make the two provider sources legible:
providers managed in the store AND providers seeded from `config.toml`
SHALL both be visible, with each row indicating its source (store-managed
vs config-seeded). A config-seeded provider SHALL NOT silently vanish from
the page while it governs session behavior; if config-seeded entries are
read-only in the page, the page SHALL say so. The New-session model catalog
and the Models page SHALL present a consistent picture of what providers
exist.

#### Scenario: config 来源的 provider 可见

- **WHEN** `config.toml` seeds a provider and the operator opens the Models
  page
- **THEN** that provider appears in the list marked as config-sourced, and
  the page indicates whether it is editable in place

#### Scenario: 创建对话框与 Models 页一致

- **WHEN** the New-session dialog's provider/model catalog is shown
  alongside the Models page
- **THEN** both surfaces reflect the same set of providers (or the Models
  page explains the difference)
