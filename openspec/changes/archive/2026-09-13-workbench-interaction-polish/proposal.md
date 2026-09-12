# 工作台交互与布局优化

## Why

工作台上线后暴露五处交互问题：界面区域不可拖拽调大小、区域间无视觉过渡；输入框工具条语义混乱（「+ new session」chip 抢占 agent 位置、settings 按钮与侧栏入口重复）；模型选择不统一且不显眼；发送按钮无状态反馈——agent 流式回复期间用户既看不到进行中状态，也无法停止一次跑偏的回复。

## What Changes

- **会话创建收拢到项目树**：项目行的「+」点击弹出创建对话框（agent 必选 + 两级模型选择 provider→model + 权限 mode 下拉，缺省「agent 默认」），替换现在「rail 自动选 agent」与「composer 创建模式」的双入口；创建后 agent 仍不可变。
- **composer 变纯跟随模式**：删除「+ new session」chip 与创建模式；左下恒显 🔒 agent 名（跟随模式下已有）。
- **删除 composer 的 settings 按钮**：侧栏底部已有同功能入口。
- **模型 chip 移到输入框右下**（挨着发送键）：单 chip + 两级分组菜单（第一层 provider、第二层该 provider 下模型）；跟随模式只列会话 `available_models`（切换走 `session/set_config_option`）；目录不可用/列表为空显示显式状态。
- **发送按钮状态机**：空输入=灰禁用；有输入=高亮；提交中=转圈；agent 流式回复中——输入框空=红色「停止」方块（点击取消当前回复，走新 cancel 链路），输入框有字=排队形态（复用 turn-queue）。
- **两道可拖拽分割线**：侧栏|主区（180–480px）、会话流|输入框（最低 120px、最高主区一半）；宽度记忆 localStorage；窄屏（<640px）退化为现状。
- **浮岛式软分隔视觉**：更深的 canvas 底色 + 各区域圆角浮岛 + 区域间留缝；分割缝 hover 亮起拖拽把手；沙箱截图迭代微调。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`：composer 工具条构成与发送按钮状态机、项目树创建对话框、布局分割与浮岛视觉基线、新增 `POST /api/sessions/{key}/cancel` 路由。
- `core-session-channel`：新增 `session.cancel` 通道方法，把 webui 的取消请求透传到会话管理既有取消机制（驱动层 interrupt 语义已存在于 acp-driver）。

## Impact

- **frontend**：`workbench-composer.ts`（工具条重构 + 状态机）、`project-rail.ts`（创建对话框）、`app-shell.ts`（分割线/浮岛框架）、`dashboard.ts`（流式态数据源）、`tokens.css`/`app.css`（视觉）、新增模型菜单与创建对话框组件。
- **backend**：`sebas-webui`（BFF cancel 路由 + 静态资源）、`src/core_channel`（新方法）、核心会话管理（透传到驱动取消）。
- **测试**：前端组件单测；testsuite e2e / acceptance 增补 cancel 与创建对话框用例。

## Non-goals

- 不做移动端专属布局（窄屏仅保证退化不破版）。
- 不做会话权限、审批流改动。
- 不动 Settings → Models 目录管理本身。
- 不做停止之外的会话控制（重启、fork、回滚等）。

## Coordination

- **实施顺序**：`rail-declutter-unread` 先行（rail 行 `...`/`+` 骨架、Inbox 分组移除），本变更随后——创建对话框接在其 `+` 上；composer 纯跟随化取代 rail 对「创建必须显式选项目」的过渡止血。
- **归属**：composer 创建模式语义（含删除）由本变更唯一拥有；`Rail session close entry` 已由 rail-declutter-unread 定义（`...` 菜单形态），本变更不再重复修改。
- **mode 控件归属**：创建时的 mode 选择在创建对话框；会话中切换留在会话头部（主规格「会话 mode 在 dashboard 可见可切」）；composer 工具条不承载 mode 控件。
