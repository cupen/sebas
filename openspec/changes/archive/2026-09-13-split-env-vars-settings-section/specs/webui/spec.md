## MODIFIED Requirements

### Requirement: 设置弹窗分区与缺省首项
设置弹窗 SHALL 暴露分区导航，分区 SHALL 按下列顺序排列：`Generic` → `Appearance` →（组间分隔线）`Services` → `Models` →（弹性留白 + 组间分隔线，压底）`Env Vars` · `About`。打开弹窗时缺省聚焦 `Generic` 分区；用户上次停留分区 SHALL 在新会话首次打开时被记住（localStorage），之后打开仍按记忆回到上次分区；记忆中的值若已不存在于分区表（如旧值 `settings`），SHALL 回退到缺省分区。

导航项 SHALL 提供足够的点击目标与选中可见性：行高约 36px、字号不低于 0.875rem、整行 hover 反馈、当前项以左侧 accent 竖条标示。

`Generic` 分区 SHALL 收敛为纯通用可配置项分区，不再承载环境变量表；在语言切换等偏好落地前，主区 SHALL 呈现说明占位文案（指明偏好项后续提供）。

`Env Vars` 分区 SHALL 承载环境变量只读表（数据来自 `GET /api/env`，见「环境变量只读展示」），与 `About` 同属底部只读参考组。

`About` 分区 SHALL 分两段呈现：INSTANCE 段在上（工作区根目录 + 复制按钮、当前 default agent kind、当前 default provider/model + 跳转 Models 分区的链接，数据来自既有 `/api/summary`、`/api/agent-defaults`），BUILD 段在下（`/api/about` 的运行时构建信息）。

原 `Settings` 总览分区移除，其维护动作「全部进程重启」与「重置 Settings」SHALL 一并移除——逐服务 restart 由 Services 分区承载，不做广播式入口。

#### Scenario: 缺省聚焦 Generic 分区

- **WHEN** 操作员从侧栏底部打开设置弹窗，且无历史记忆
- **THEN** 弹窗打开后左导航高亮 `Generic`，主区渲染 `Generic` 分区内容（占位文案）

#### Scenario: 历史记忆恢复上次分区

- **WHEN** 操作员上次停留在 `Services` 后关闭弹窗，再打开
- **THEN** 弹窗缺省聚焦 `Services` 分区

#### Scenario: 缺省聚焦 Settings 分区

- **WHEN** localStorage 记忆值为 `settings`（本变更前的合法分区名，`Settings` 总览分区已由 `Generic` 接替）
- **THEN** 左导航高亮 `Generic` 而非报错或空白

#### Scenario: 分区顺序与底部只读组

- **WHEN** 设置弹窗渲染左导航
- **THEN** 分区按 `Generic → Appearance → Services → Models → Env Vars · About` 排列，`Appearance` 与 `Services` 之间有组间分隔线；`Env Vars` 与 `About` 通过弹性留白压在导航底部、上方有分隔线，与功能区视觉分离

#### Scenario: 分区顺序与 About 压底

- **WHEN** 设置弹窗渲染左导航
- **THEN** 功能分区按 `Generic → Appearance → Services → Models` 排列，`About` 与 `Env Vars` 同处底部只读组、整体通过弹性留白压底且上方有分隔线

#### Scenario: Settings 分区总览

- **WHEN** 聚焦 `About` 分区
- **THEN** 主区先呈现 INSTANCE 段——工作区根目录（路径 + 复制按钮）、default agent kind（只读）、default provider/model（只读 + 跳转 Models 分区的链接），后呈现 BUILD 段（版本、commit、构建时间）

#### Scenario: Generic 分区不再有环境变量表

- **WHEN** 聚焦 `Generic` 分区
- **THEN** 主区呈现偏好占位文案，环境变量只读表不再出现在该分区

#### Scenario: 环境变量表移居 Env Vars 分区

- **WHEN** 聚焦 `Env Vars` 分区
- **THEN** 主区渲染环境变量只读表（名字、解释、按分类展示的值），数据来自 `GET /api/env`

#### Scenario: About 分区承载实例信息

- **WHEN** 聚焦 `About` 分区
- **THEN** 主区先呈现 INSTANCE 段（工作区根目录 + 复制按钮、default agent kind、default provider/model + 跳转 Models 链接），后呈现 BUILD 段（版本、commit、构建时间）

#### Scenario: 高危动作二次确认

- **WHEN** 设置弹窗任意分区渲染
- **THEN** 不存在「全部进程重启」与「重置 Settings」入口——原高危动作已随 `Settings` 总览分区删除，确认对话框随之消失；逐服务 restart 只在 Services 分区出现

## ADDED Requirements

### Requirement: 环境变量只读展示（/api/env）

WebUI SHALL 提供只读端点 `GET /api/env`：读取 **webui 进程自身**的环境变量，返回**策划过**的变量清单（每项含名字、人读解释、按分类的值展示），经既有鉴权。策划清单之外的环境变量（内部管道、测试专用）SHALL NOT 出现在响应中。清单分类与展示语义：

- **非敏感**（路径与开关，如 `SEBAS_STATE_DB`、`SEBAS_STATE_FILE`、`SEBAS_ROUTER_CONFIG`、`SEBAS_ROUTER_PROVIDER_OVERLAY`、`SEBAS_LOG_LEVEL`、`SEBAS_HANG_TIMEOUT_SECS`、`SEBAS_FEISHU_APP_ID`）：已设置显示实际值；未设置标注「未设置（用默认）」且解释里写明默认值。
- **敏感**（凭据类，如 `SEBAS_WEBUI_PASSWORD`、`SEBAS_WEBUI_TOKEN`、`SEBAS_CONTROL_SECRET`、`SEBAS_FEISHU_APP_SECRET`）：SHALL 只显示「已设置 / 未设置」，**值本身 SHALL NOT 出现在响应里**（遮蔽在服务端完成）。

前端 `Env Vars` 分区 SHALL 消费该端点渲染只读表；端点失败（core 无关，纯 webui 面）时分区 SHALL 如实呈现错误状态而非空白或编造数据。

#### Scenario: 非敏感变量显示实际值或默认标注

- **WHEN** `GET /api/env` 返回且 `SEBAS_STATE_DB` 已设置
- **THEN** 该项显示实际路径值；`SEBAS_STATE_FILE` 未设置时该项显示「未设置（用默认）」且解释含默认路径

#### Scenario: 敏感值永不出现在响应

- **WHEN** `SEBAS_FEISHU_APP_SECRET` 已设置且操作员请求 `GET /api/env`
- **THEN** 响应中该项只标「已设置」，任何字段都不含其值明文

#### Scenario: 内部与测试变量不列

- **WHEN** webui 进程环境中存在内部管道或测试专用变量（如 `SEBAS_IPC`、`SEBAS_TEST_SPAWN_SESSION`）
- **THEN** 它们不出现在 `/api/env` 响应与 Env Vars 分区表中

#### Scenario: 端点经既有鉴权

- **WHEN** webui 鉴权开启且未登录的客户端请求 `GET /api/env`
- **THEN** 请求被既有鉴权拒绝，与其他 `/api` 端点一致

#### Scenario: 端点失败如实呈现

- **WHEN** `GET /api/env` 请求失败（非 200）
- **THEN** Env Vars 分区呈现明确的错误提示，不渲染编造或空表假象
