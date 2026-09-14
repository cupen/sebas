## Purpose

定义工作区根目录（workspace root）：每台机器一个项目目录 containment 边界，约定其取值解析顺序与路径范围判定语义，供 WebUI 项目面与执行节点共同引用。

## ADDED Requirements

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

范围判定 SHALL 把候选路径与 workspace root 都解析到真实路径（canonicalize）后做逐分量前缀比较。候选路径无法解析时 SHALL 判为越界；workspace root 自身无法解析时 SHALL 判定一切候选路径越界（fail-closed）。符号链接逃逸（根内路径经链接指向根外）SHALL 判为越界。

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

### Requirement: 每台机器各自生效、互不影响

workspace root SHALL 是每机器独立的：控制平面（WebUI 所在机器）以自己的 root 约束本机项目面；执行节点以自己的 root 约束其上的路径判定。二者取值可以不同，任何一侧的取值 SHALL NOT 影响另一侧的判定。

#### Scenario: 节点根与主控根不同

- **WHEN** 控制平面 workspace root 为 `/home/op/work`，某执行节点 workspace root 为 `/srv/nodes/a/work`
- **THEN** 主控按 `/home/op/work` 判定本机注册，节点按 `/srv/nodes/a/work` 判定远端注册，互不引用对方取值

#### Scenario: 节点未配置不影响主控

- **WHEN** 某执行节点未显式配置 workspace root
- **THEN** 仅该节点回退到其自身当前工作目录并告警，控制平面的判定不受影响
