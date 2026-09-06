# webui-browser-e2e Specification（增量）

## Purpose

用 Playwright 在真实浏览器（chromium）里驱动 sebas webui 的旅程级 e2e 套件：后端为一次性沙箱（AGENTS.md 调试菜谱形态 + fake-claude 桩），补齐验收账本中"浏览器级 UI 渲染"豁免面的自动化回归能力。

## ADDED Requirements

### Requirement: 沙箱装配与清理边界

套件 SHALL 以一次性沙箱目录装配被测后端：`sebas core --router --debug --webui` 单进程形态，agent 为仓库自带 fake-claude 桩；MUST 覆盖全部默认路径与凭据 env（state DB、state file、provider overlay、core secret、auth file），MUST NOT 绑定 9797 或读写真实 `~/.sebas`。套件结束（无论成败）SHALL 结束后端进程并删除沙箱目录——POSIX 上后端 SHALL 优雅退出（SIGTERM），Windows 上允许硬终止、目录删除尽力而为；失败时 SHALL 保留现场目录并在输出中给出路径，供排障复用。

#### Scenario: 启动即隔离

- **WHEN** 套件拉起后端
- **THEN** 全部状态（config、DB、media、ACP 会话目录、auth 文件）位于一次性沙箱目录，webui 端口 ≠ 9797，进程 env 不含指向真实 `~/.sebas` 的路径

#### Scenario: 成功退出清理

- **WHEN** 全部用例通过、套件退出
- **THEN** 后端进程结束、沙箱目录被删除、端口恢复可用；POSIX 上后端为 SIGTERM 优雅退出，Windows 上允许硬终止（清理尽力而为）

#### Scenario: 失败保留现场

- **WHEN** 任一用例失败
- **THEN** 套件报告失败并在输出中保留沙箱目录路径，目录内含后端日志可供复现

### Requirement: 会话核心旅程

套件 SHALL 在浏览器中覆盖会话核心旅程：从 workbench composer 发起会话（ACP 后端）→ 用户消息与桩回复按序出现在 transcript → 回合结束状态收敛为完成；流式回合 SHALL 可观测到内容分批到达与运行中的瞬态状态；页面重载后 transcript SHALL 从持久化状态恢复；桩触发"拒绝"时 SHALL 呈现非终态错误且会话存活（下一条消息仍可用），触发"崩溃"时 SHALL 如实呈现子进程死亡。

#### Scenario: 首回合往返

- **WHEN** 操作员在 composer 输入文本提交
- **THEN** transcript 依次出现用户消息与桩的固定回复，会话状态收敛为 Done

#### Scenario: 流式分批渲染

- **WHEN** 以流式触发词发起回合
- **THEN** 内容分批出现且中途存在运行中的瞬态状态，最终回合完成

#### Scenario: 重载恢复

- **WHEN** 完成一回合后刷新页面并回到该会话
- **THEN** transcript 与状态从持久化恢复，内容不丢

#### Scenario: 错误呈现

- **WHEN** 桩触发非终态拒绝与子进程崩溃两种错误
- **THEN** 前者在 UI 呈现错误且会话继续可用，后者如实呈现死亡而非伪装成功

### Requirement: 审批卡片旅程

套件 SHALL 覆盖 gated tool call 的浏览器审批闭环：桩发起需审批的工具调用时 review card SHALL 出现；allow-once 后工具结果以允许语义呈现；deny 后工具以拒绝语义呈现且回合结束；allow-session 后同一会话的同类后续工具调用不再弹卡。全部决策 SHALL 经真实审批 API 回传，fail-closed 语义保持（不决策则不执行）。

#### Scenario: 拒绝路径

- **WHEN** review card 出现后操作员点击 deny
- **THEN** 卡片消失，transcript 呈现拒绝语义的工具结果，回合完成

#### Scenario: 单次允许路径

- **WHEN** 操作员点击 allow-once
- **THEN** 工具结果以允许语义呈现，回合完成

#### Scenario: 会话级允许

- **WHEN** 操作员点击 allow-session 后再次触发同类工具调用
- **THEN** 不再出现 review card，工具直接执行

### Requirement: 鉴权与访问旅程

套件 SHALL 在鉴权关闭与开启两种形态下分别验证：关闭时应用免登录直达 workbench；开启时未登录访问被重定向到登录页、错误凭据被拒绝、admin/admin 登录成功进入 workbench、登出后回到未鉴权态。沙箱装配 SHALL 支持以参数切换两种形态（统一测试账户 admin/admin）。

#### Scenario: 免登录直达

- **WHEN** 鉴权关闭时打开根路径
- **THEN** 直接渲染 workbench，无登录页

#### Scenario: 登录闭环

- **WHEN** 鉴权开启时以 admin/admin 登录、登出
- **THEN** 登录后进入 workbench，登出后再次访问需重新登录；错误凭据被拒绝且不进入应用

### Requirement: 工作台、项目与会话管理面旅程

套件 SHALL 覆盖：workbench 首屏结构（项目栏、composer、summary/reachability 如实显示沙箱真实状态）；项目添加与移除在项目栏即时生效；会话 close 后从活动列表消失；archive 后从列表隐藏且归档视图可见该条目；会话深链在 SPA fallback 下可直达；退役路径（如 /settings）canonical 重定向到 `/`；无模型选项的 ACP 会话不显示模型下拉且不报错（D4 诚实缺省）。

#### Scenario: 首屏与 reachability

- **WHEN** 打开根路径
- **THEN** 项目栏、composer、summary 呈现，reachability 显示沙箱内各执行体的真实状态

#### Scenario: 项目增删

- **WHEN** 添加一个沙箱内目录为项目再将其移除
- **THEN** 项目栏先后呈现与移除该项目

#### Scenario: close 与 archive

- **WHEN** 对已有会话先后执行 close 与 archive
- **THEN** close 后会话离开活动列表；archive 后归档视图可见该条目、活动列表不再显示

#### Scenario: 深链与退役路径

- **WHEN** 直接访问会话深链与退役路径
- **THEN** 深链经 SPA fallback 渲染对应会话，退役路径重定向到 `/`

#### Scenario: 模型面诚实缺省

- **WHEN** 打开无模型选项的桩会话
- **THEN** 模型下拉不渲染、无控制台错误，模型切换请求得到如实的拒绝语义

### Requirement: 一键入口与稳定性

`invoke webui-e2e` SHALL 是套件唯一入口：构建（含 dist 自动重建）→ 装配沙箱 → 运行全部用例 → 清理；支持按名过滤单个旅程。断言 SHALL 基于状态轮询而非固定 sleep；同一提交重复运行 SHALL 稳定收敛（无依赖时序的偶发失败）。

#### Scenario: 一键运行

- **WHEN** 在仓库根执行 `invoke webui-e2e`
- **THEN** 无需人工步骤完成构建、运行与清理，退出码反映通过与否

#### Scenario: 单旅程过滤

- **WHEN** 以旅程名过滤执行
- **THEN** 仅运行匹配的 spec 文件，清理行为与全量运行一致
