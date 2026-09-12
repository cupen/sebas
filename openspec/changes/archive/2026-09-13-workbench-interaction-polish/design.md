# 设计：工作台交互与布局优化

## Context

工作台现状：app-shell 是写死 220px 的侧栏 + main 的 flex 框架（无 splitter）；composer（`workbench-composer.ts`）身兼两职（跟随 + 创建），工具条上有 `+ new session` chip、`settings →`、创建模式的三 select；发送按钮只有 enabled/disabled 两态；后端无任何 cancel HTTP API，但驱动层已有中断语义（acp-driver "Cancel via interrupt and respawn-with-resume"，`session.rs` 的 `cancel_tx`）。Web Awesome 已在依赖中且自带 `wa-split-panel`。动机见 proposal.md。

## Goals / Non-Goals

Goals：单一创建入口（rail 对话框）；composer 纯跟随化 + 工具条语义收敛；发送按钮完整状态机（含停止）；两道可拖拽分割线；浮岛视觉基线。Non-goals 见 proposal.md（移动端专属布局、审批流、Models 管理页、停止之外的会话控制）。

## Decisions

**D1 — 分割线用 `wa-split-panel`，不手写。** 已在依赖树里（`@awesome.me/webawesome` dist/components/split-panel），自带键盘可达性与 pointer 处理。侧栏|主区用 horizontal 实例放进 app-shell（替换 `flex: 0 0 auto; width: 220px` 的 nav 定宽），stage|composer 用 vertical 实例放进 dashboard。定位是百分比而边界是 px（侧栏 180–480px）：监听 input 事件换算成 px、clamp、写 localStorage（`sebas.rail-width` / `sebas.composer-height`），初始化时从 localStorage 读回换算成 position；读写都 try/catch（隐私模式兜底）。<640px 媒体查询内隐藏分割行为（现状已是纵向堆叠布局）。备选：手写 pointer-event splitter（多 ~100 行、要自己补 a11y）——否决。

**D2 — 创建对话框是新组件 `sebas-new-session-dialog`，由 project-rail 持有。** wa-dialog 承载：agent 下拉（必选，`/api/agents`，预选项目 default_agent——rail 已有该数据）+ 两级模型选择（provider→model）。模型目录加载逻辑从 composer 抽成 `model-catalog.ts` 里的 `loadModelCatalog()`（providers + defaults 并取、toModelCatalog、预选规则），对话框与 composer 芯片共用，避免双份 fetch 逻辑漂移。创建入口只有项目行的「+」（Inbox 分组由先行的 `rail-declutter-unread` 移除，本变更不再提供无项目创建位）。确认后调既有 `POST /api/sessions`（0-turn 占位，agent 必填），成功后沿用 create 后的 set_focus 链路：rail 通知 app-shell/dashboard refetch。取消则什么都不发生。composer 删除创建模式：`createRequested`、agent select、provider select、`+ new session` chip、`settings-link` 全部移除；创建模式的 `mode-chip` 一并移除——创建时 mode 选择进对话框（`ask | edit | allow | auto`，缺省「agent 默认」不发送 mode 字段），会话中切换留在会话头部。

**D3 — 模型 chip 的 provider 分组靠目录交叉引用。** 跟随模式的 `available_models` 是平铺 id，不带 provider 归属；用户已拍板要两级菜单。方案：用目录（`loadModelCatalog()` 结果）把每个 model id 映射回 provider 完成分组；目录里查不到的 id 归入"会话提供"组置底；目录整体不可得时退化为单层平铺（仍可用，不伪造分组）。当前模型打勾标记。位置在 composer 底部右侧、发送键左侧。

**D4 — 发送状态机由 dashboard 供数据、composer 呈现。** dashboard 已经持有聚焦会话的 status（summary + WS 推送），新增 prop `turnInFlight`（status == Working）传入 composer。优先级：POST 在途（转圈）> turnInFlight && text==""（红色方块停止）> turnInFlight && text!=""（排队形态，提交走既有 turn-queue）> text!=""（send）> disabled。停止点击调新增 `api.cancelSession(key)`，错误走既有 `.callout-error`；turn 结束（WS 推送）自动回到 send 态。排队形态复用 pending-stack 的既有语义，不新增 wire 字段。

**D5 — cancel 链路：webui BFF → core channel `session.cancel` → 会话管理 → 既有取消机制。** 新路由 `POST /api/sessions/{key}/cancel`（POST-only，鉴权/mutation 姿态与 message 同）；core_channel 客户端与服务端各加一个方法；核心侧找到该 key 的活动执行体后触发驱动层既有 cancel（interrupt），子进程与会话存活（符合 acp-driver 的 interrupt-and-heal）。Typed rejections：未知 key、无在途 turn（idle）。**已知取舍**：native 内核若无中断实现则返回 typed "cancel unsupported" 拒绝——UI 不预判执行体，停止按钮照常出现，失败原因经 callout 如实呈现。queued 提交不被 cancel 丢弃（下一个 turn 继续执行）。

**D6 — 浮岛视觉在 tokens 层落地。** `tokens.css` 新增 canvas 底色 token（比 surface 深一档），app-shell 背景换 canvas；rail、stage、composer 变圆角浮岛（现 composer 已是 18px 圆角卡片，向它看齐），区域间用间距替代通高 1px 硬线（移除 composer 底部的 `hr.divider`）；分割线 rest 态透明、hover 亮起把手（`wa-split-panel` 的 divider part 定制）。遵守 reduced-motion 既有约定。视觉验收走 `invoke testsuite-webui-sandbox` 截图迭代，不一次性定死。

## Risks / Trade-offs

- [wa-split-panel 与 sticky 侧栏 footer、100vh 框架的相互影响] → 先在沙箱里做布局原型再迁移其余样式；失败兜底是手写 splitter（D1 备选）。
- [操作者真机 AppImage 烘焙的是旧 dist，新 UI 不可见] → 交付说明里明确需要重新 `cargo build`/打包；沙箱验证与真机验证分开陈述。
- [available_models 无 provider 归属，分组是推断] → 交叉引用失败有显式兜底组（D3），不伪造归属。
- [native 内核 cancel 可能无实现] → typed 拒绝 + callout 呈现（D5），UI 不隐藏停止按钮。
- [localStorage 不可用（隐私模式）] → 读写 try/catch，退化为默认尺寸。

## Migration Plan

无数据迁移、无配置变更。纯代码发布：前端构建随 `cargo build` 烘焙。回滚 = revert 提交。旧 dist 烘焙的实例在升级前继续服务旧 UI，无兼容断点。

## Open Questions

无阻塞项。实现期待确认的细节（不改变规格）：wa-split-panel 事件换算的精度处理；停止按钮的具体图标与红红色阶取值（沙箱截图迭代时定）。
