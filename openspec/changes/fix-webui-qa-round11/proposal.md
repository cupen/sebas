# fix-webui-qa-round11 Proposal

## Why

第十一轮 GUI 全功能验收（QA-A 认证/设置/项目/技能簇 + QA-B 会话核心链路簇，证据在
`C:\Users\cupen\AppData\Local\Temp\sebas-qa-r11\findings-a.md` 与 `findings-b.md`，
沙箱 fake-claude + native `test/*` 场景模型）确认 11 个缺陷：2 个 P1——viewer 角色
可删除技能仓条目（服务端未拒绝 + 删除控件对只读角色可见，对照：同角色注册项目被
明确拒绝）、native 会话生命周期状态恒为 Queued 永不推进（连带活跃/休眠计数失真与
会话标题不自动命名，同源于 native 路径元数据不回填）；3 个 P2——native 流式回合无
停止入口、后台完成的回合无任何通知、零 store provider 时新建别名的目标下拉为空且
无指引；6 个 P3——多行消息气泡渲染为单行、已关闭会话深链持续 404 轮询、未登录态
WS 秒级重连刷认证失败（QA-A A-3 与 QA-B B-6 同根，合并处理）、设置弹窗失焦时不响
应 Esc、usage 折线图最右刻度被截断、删除 agent 确认文案断句歧义。主链路（登录、
RBAC 隔离、项目、会话、审批四档、thinking、流式、异常/空回合、用量对账、持久化、
主题）全部验证可用。

## What Changes

- 技能删除角色门禁：技能条目移除限 root/admin（复用 round10 接线的 `settings.manage`
  权限键），服务端路由守卫强制 + 前端技能页对只读角色隐藏删除控件；rbac 矩阵本身不动
- native 会话生命周期回填：native 体把卡相位（OnIt/Done/CrossMark）与标题回填进同一
  元数据通道，状态派生保持 `SessionStatus::derive` 单点；修复后 native 会话推进
  Working/Done/Failed、顶部活跃/休眠计数如实、首条消息自动命名（与 ACP 同语义）
- native 流式停止入口：composer 的停止/取消控件对 native in-flight 回合同样出现，
  取消语义沿用 session-lifecycle 既有要求（即时反馈、终态落转录）
- 回合完成通知接线：会话非聚焦时回合完成/失败经既有分级通知层发 info 级通知；
  聚焦中的会话不弹（避免噪音）
- 别名空下拉指引：零 store provider 时新建别名的目标下拉呈禁用态 + 「请先在模型分区
  新建 provider」指引文案
- 多行气泡渲染：消息气泡 white-space 按 pre-wrap 保留换行（wire 往返已含 `\n`，
  纯渲染层修复）
- 深链止停轮询：会话不存在/已关闭时首次 404 即止停轮询，保留现有「会话不可得」清晰呈现
- WS 认证态门禁：SPA 未认证（登录页/登出态/首启设置页）不发起 `/ws` 连接；认证失效
  关闭后不自动重连，登录成功后再建连
- 设置弹窗 Esc：弹窗打开期间窗口级 Esc 一律关闭（不再依赖焦点在弹窗内）
- usage 图表刻度：折线图右缘补内边距，最右 x 轴标签完整可见
- 删除 agent 确认文案：消除「下拉 中立即可见」断句歧义，统一空格

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-skills`: webui 移除窗口的角色执法——删除动作限 root/admin，只读角色无写控件
- `session-lifecycle`: native 会话的状态推进、计数如实与自动命名；停止可供性覆盖
  native in-flight 回合
- `webui`: 分级通知层接入回合完成/失败事件（非聚焦会话）；未认证态的 WS 连接门禁
- `provider-management`: 零 provider 时别名创建的空态指引
- `agent-workbench`: 多行消息气泡保留换行；已关闭会话深链止停轮询
- `agent-settings`: 设置弹窗失焦 Esc 关闭；删除 agent 确认文案消歧
- `usage-statistics`: usage 折线图右缘刻度完整呈现

## Impact

- `sebas-webui/src/routes.rs`（技能删除路由挂 `settings.manage` 守卫）、native 体元数据
  回填路径（卡相位/标题写入会话映射，嫌疑区 `src/agent_backend.rs` native 装配面与会话
  状态派生输入 `sebas-webui/src/models.rs` 保持不动）
- `sebas-webui/frontend/src/views/`（settings-modal.ts 技能页写控件与 Esc、
  transcript-view.ts 气泡 white-space、workbench-composer.ts 停止控件与深链轮询、
  settings-aliases.ts 空态指引、usage.ts 图表边距、app-shell.ts WS 门禁、
  role-visibility.ts 技能页角色隐藏、notify.ts 事件接线）
- `tests/`（技能删除守卫 API 测试、native 状态推进回归、WS 门禁前端单测、渲染回归）
- 无 wire 形状破坏性变更；全部为行为修复、门禁收紧与呈现修复

## Non-goals

- bash 工具的 Windows 平台限制（`io error: unix only`）：平台能力边界，记录观察项不修
- tools-parallel 审批卡「队列串行 + 末尾并排」形态调整：机制可用、互不干扰已验证
- usage 只统计 router 透传流量、ACP 回合不入图的口径：架构现状，如实呈现
- native 会话 key 的「飞书 ·」前缀标签：显示层历史命名，留观察项
- rbac.rs 四档权限矩阵本身的重排（只复用既有 `settings.manage` 键接线）
