## MODIFIED Requirements

### Requirement: 设置弹窗分区与缺省首项
设置弹窗 SHALL 暴露分区导航，分区 SHALL 按下列顺序排列：`Generic` → `Appearance` →（组间分隔线）`Services` → `Models` →（弹性留白 + 组间分隔线，压底）`About`。打开弹窗时缺省聚焦 `Generic` 分区；用户上次停留分区 SHALL 在新会话首次打开时被记住（localStorage），之后打开仍按记忆回到上次分区；记忆中的值若已不存在于分区表（如旧值 `settings`），SHALL 回退到缺省分区。

导航项 SHALL 提供足够的点击目标与选中可见性：行高约 36px、字号不低于 0.875rem、整行 hover 反馈、当前项以左侧 accent 竖条标示。

`Generic` 分区 SHALL 承载通用偏好与杂项——初期内容为原 `Env` 分区的环境变量只读表，并为后续语言切换等偏好预留信息架构位置。

`About` 分区 SHALL 分两段呈现：INSTANCE 段在上（工作区根目录 + 复制按钮、当前 default agent kind、当前 default provider/model + 跳转 Models 分区的链接，数据来自既有 `/api/summary`、`/api/agent-defaults`），BUILD 段在下（`/api/about` 的运行时构建信息）。

原 `Settings` 总览分区移除，其维护动作「全部进程重启」与「重置 Settings」SHALL 一并移除——逐服务 restart 由 Services 分区承载，不做广播式入口。

#### Scenario: 缺省聚焦 Generic 分区

- **WHEN** 操作员从侧栏底部打开设置弹窗，且无历史记忆
- **THEN** 弹窗打开后左导航高亮 `Generic`，主区渲染 `Generic` 分区内容

#### Scenario: 历史记忆恢复上次分区

- **WHEN** 操作员上次停留在 `Services` 后关闭弹窗，再打开
- **THEN** 弹窗缺省聚焦 `Services` 分区

#### Scenario: 缺省聚焦 Settings 分区

- **WHEN** localStorage 记忆值为 `settings`（本变更前的合法分区名，`Settings` 总览分区已由 `Generic` 接替）
- **THEN** 左导航高亮 `Generic` 而非报错或空白

#### Scenario: 分区顺序与 About 压底

- **WHEN** 设置弹窗渲染左导航
- **THEN** 分区按 `Generic → Appearance → Services → Models` 顺序排列，`Appearance` 与 `Services` 之间有组间分隔线；`About` 通过弹性留白压在导航底部、上方有分隔线，与功能区视觉分离

#### Scenario: Settings 分区总览

- **WHEN** 聚焦 `About` 分区
- **THEN** 主区先呈现 INSTANCE 段——即原 Settings 总览的三只读项：工作区根目录（路径 + 复制按钮）、default agent kind（只读）、default provider/model（只读 + 跳转 Models 分区的链接），后呈现 BUILD 段（版本、commit、构建时间）

#### Scenario: About 分区承载实例信息

- **WHEN** 聚焦 `About` 分区
- **THEN** 主区先呈现 INSTANCE 段（工作区根目录 + 复制按钮、default agent kind、default provider/model + 跳转 Models 链接），后呈现 BUILD 段（版本、commit、构建时间）

#### Scenario: 高危动作二次确认

- **WHEN** 设置弹窗任意分区渲染
- **THEN** 不存在「全部进程重启」与「重置 Settings」入口——原高危动作已随 `Settings` 总览分区删除，确认对话框随之消失；逐服务 restart 只在 Services 分区出现

### Requirement: Fetch models from the provider's official base URL

The WebUI provider surface SHALL offer a fetch action that retrieves the model ids the
provider's official base URL currently serves. The action SHALL live inside the
provider editor, next to the Models block heading — the provider row SHALL NOT carry
a fetch button. It SHALL be available for preset-derived and custom providers alike
whose provider has a usable base URL, and SHALL be hidden when there is none. Fetching
SHALL persist nothing by itself: the returned ids replace the editor's in-memory model
list wholesale (deduplicated by id; an existing entry whose id also appears in the
fetched list keeps its manually assigned capability tags), and the replacement reaches
the stored provider only through the editor's normal save. A failed fetch SHALL leave
the editor's model list untouched and report the sanitized reason; a failure SHALL NOT
be presented as an empty successful list.

#### Scenario: fetch lists the official model ids

- **WHEN** the operator opens the provider editor and runs fetch on a provider whose
  base URL serves a model list
- **THEN** the returned ids replace the editor's in-memory model list, and the
  provider's stored data is unchanged until the operator saves the editor

#### Scenario: fetch lists the official model ids into the editor

- **WHEN** the operator opens the provider editor and runs fetch on a provider whose
  base URL serves model ids `m1`, `m2`
- **THEN** the editor's model list is replaced by `m1`, `m2`, and the provider's
  stored data is unchanged until the operator saves the editor

#### Scenario: picking a fetched model edits the list

- **WHEN** a fetch returns model ids and the operator saves the editor
- **THEN** the fetched ids join the provider's stored model list through the ordinary
  editor save, each with the implicit text capability and no invented parameters

#### Scenario: picking tags is preserved for surviving ids

- **WHEN** the editor lists model `m1` tagged `vision` and a fetch returns `m1`, `m2`
- **THEN** after the replacement `m1` still carries its `vision` tag and `m2` starts
  with no capability tags beyond the implicit text capability

#### Scenario: cancelling the editor discards the fetch

- **WHEN** the operator runs fetch and then closes the editor without saving
- **THEN** the provider's stored model list is unchanged

#### Scenario: no base URL means no fetch entry

- **WHEN** a provider has no usable base URL
- **THEN** the editor renders no fetch action for it

#### Scenario: failure is reported honestly

- **WHEN** the upstream fetch fails
- **THEN** the editor's model list keeps its prior content and the surface shows the
  sanitized reason, not an empty list presented as success

