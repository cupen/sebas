# workspace-root Specification

## Purpose
定义工作区根目录（workspace root）：每台机器一个项目目录 containment 边界，约定其取值解析顺序与路径范围判定语义，供 WebUI 项目面与执行节点共同引用。

## Requirements

### Requirement: Workspace root 的解析顺序

每台机器 SHALL 恰有一个生效的 workspace root，按以下顺序解析：`SEBAS_WORKSPACE_ROOT` 环境变量 > 本机配置项 > 回退进程当前工作目录。当回退到进程当前工作目录时，进程 SHALL 在启动时输出一条告警日志，提示显式配置 workspace root。workspace root SHALL 始终存在（不存在「未启用约束」的运行形态）。

#### Scenario: 环境变量优先

- **WHEN** 进程环境设置了 `SEBAS_WORKSPACE_ROOT` 且配置文件也配置了 workspace root
- **THEN** 生效的是环境变量指向的目录，且不输出回退告警

#### Scenario: 配置项次之

- **WHEN** 环境变量未设置且配置文件配置了 workspace root
- **THEN** 生效的是配置项指向的目录，且不输出回退告警

#### Scenario: 回退当前目录并告警

- **WHEN** 环境变量与配置项均未提供 workspace root
- **THEN** 生效的是进程当前工作目录，且启动日志包含一条要求显式配置的告警

### Requirement: 范围判定是规范化且 fail-closed 的

范围判定 SHALL 把候选路径与 workspace root 都解析到真实路径（canonicalize）后做逐分量前缀比较。候选路径无法解析时 SHALL 判为越界；workspace root 自身无法解析时 SHALL 判定一切候选路径越界（fail-closed）。符号链接逃逸（根内路径经链接指向根外）SHALL 判为越界。判定拒绝 SHALL 面向操作者呈现具体的拒绝原因（如「路径在 workspace root 之外」「路径不存在」），注册界面 SHALL 在给出原因的同时保持提交控件禁用，不允许只禁用不解释。

#### Scenario: 符号链接逃逸被拒绝

- **WHEN** workspace root 内的某个目录是指向根外目标的符号链接
- **AND** 以该目录作为项目路径发起注册或打开
- **THEN** 判定为越界并被拒绝

#### Scenario: 相对路径穿越被拒绝

- **WHEN** 候选路径携带 `..` 分量且解析后落在 workspace root 之外
- **THEN** 判定为越界

#### Scenario: 无法解析即越界

- **WHEN** 候选路径不存在或无法 canonicalize
- **THEN** 判定为越界（与确定的越界同罪，不区分文案）

#### Scenario: 手填越界路径给出禁用原因

- **WHEN** 操作者在注册界面手填一个 workspace root 之外（或不存在）的路径
- **THEN** 界面在路径输入下方显示具体禁用原因，且提交控件保持禁用

### Requirement: 每台机器各自生效、互不影响

workspace root SHALL 是每机器独立的：控制平面（WebUI 所在机器）以自己的 root 约束本机项目面；执行节点以自己的 root 约束其上的路径判定。二者取值可以不同，任何一侧的取值 SHALL NOT 影响另一侧的判定。

#### Scenario: 节点根与主控根不同

- **WHEN** 控制平面 workspace root 为 `/home/op/work`，某执行节点 workspace root 为 `/srv/nodes/a/work`
- **THEN** 主控按 `/home/op/work` 判定本机注册，节点按 `/srv/nodes/a/work` 判定远端注册，互不引用对方取值

#### Scenario: 节点未配置不影响主控

- **WHEN** 某执行节点未显式配置 workspace root
- **THEN** 仅该节点回退到其自身当前工作目录并告警，控制平面的判定不受影响

### Requirement: Project registry is persisted in the core state store

The project registry — local projects and projects placed on remote execution nodes alike — SHALL be persisted in the core state store, which SHALL be its only authority. The registry SHALL carry the node dimension for every entry, so that a project's placement is part of its persisted identity rather than a property that exists only on disk somewhere else. Only the core process SHALL write the registry; every other role SHALL read and mutate it through the state methods. No separate project-registry file SHALL be written or read.

#### Scenario: a remote project survives a restart

- **WHEN** a project is added for a remote execution node and the process restarts
- **THEN** the project is present in the registry with the same node placement
- **AND** it was read from the state store, not from a file

#### Scenario: local and remote projects share one store

- **WHEN** both a local project and a remote-node project are registered
- **THEN** both are read from the state store in one listing
- **AND** no project entry is read from a separate file

#### Scenario: no project-registry file is written

- **WHEN** projects are added, renamed, reordered, or removed
- **THEN** no project-registry file is created or modified on disk

#### Scenario: placement is honoured from the persisted registry

- **WHEN** the registry is read after a restart
- **THEN** each entry's node placement is taken from the store
- **AND** a local project is distinguishable from one placed on a node

#### Scenario: a local project keeps its existing serialized spelling

- **WHEN** a local project is listed through any surface
- **THEN** its node placement serializes exactly as it did before the placement became a persisted column
- **AND** a client that does not care about placement sees an unchanged shape

### Requirement: Project surfaces degrade honestly when the store is unreachable

A surface that renders projects SHALL present an explicit unavailable state naming the cause when the state store cannot be reached. It SHALL NOT fall back to a file-derived project list, and SHALL NOT present a file-derived or previously cached list as the current registry. Mutating entries SHALL be disabled while the store is unavailable.

#### Scenario: unreachable store shows unavailable, not a file

- **WHEN** the core is not running and a project surface is opened
- **THEN** it presents an explicit unavailable state with the cause
- **AND** it does not show a project list read from a file

#### Scenario: mutations are disabled while unavailable

- **WHEN** the store is unreachable
- **THEN** project add, rename, reorder, and remove entry points are disabled
- **AND** no local file is written as a substitute

#### Scenario: recovery restores the registry

- **WHEN** the store becomes reachable again
- **THEN** the surface shows the registry read from the store
- **AND** the unavailable state is cleared

### Requirement: A project record has one canonical shape

A project record SHALL have exactly one canonical definition, used for both its persisted form and its wire form. A consumer SHALL NOT re-list the record's fields in a parallel declaration, and the two forms SHALL NOT diverge: the set of fields and their serialized names SHALL be stated in one place and pinned by a test, so that adding a field cannot update only one side.

#### Scenario: storage and wire shapes cannot drift

- **WHEN** a field is added to, removed from, or renamed in the project record
- **THEN** the change is made in the single definition
- **AND** the pinning test reflects both the persisted and the wire form, so a one-sided change fails

#### Scenario: no parallel field listing remains

- **WHEN** the workspace is searched for declarations of a project record
- **THEN** exactly one definition exists for the shared fields
- **AND** no consumer carries a hand-written field-by-field copy of it

#### Scenario: presentation-only fields stay out of the record

- **WHEN** a surface needs a display-only field (such as a computed label)
- **THEN** that field is produced by a conversion from the canonical record rather than added to it
- **AND** the record does not grow to carry presentation concerns
