## 1. 复合后端 pending 管理面转发（D2）

- [x] 1.1 `DualSessionBackend` 实现 `pending` / `remove_pending` / `move_pending`，复用 `route()` 按 key 转发到承载子后端（acp 桥）；native 会话得到诚实类型化拒绝。验证标准：单测断言 acp 会话的 move/remove 到达 acp 桥、native 会话返回 Unavailable（src/agent_backend.rs 既有测试形态）
- [x] 1.2 核对 `sebas-webui/src/api.rs` 的拒绝映射：Unavailable 保持 503，PendingRejected 四类（Unknown/AlreadyStarted/PriorityConflict/OutOfRange）保持 404/409/409/400。验证标准：api.rs 路由层 fake 测试（既有形态）覆盖四类透传
  - 状态：映射本就正确（无需改动）；FakeBackend 新增 `set_pending_op_rejection` 缝，api.rs 路由测试覆盖四类透传 + 缺省 Unavailable 503。
- [x] 1.3 前端 notice-layer：`Unavailable` 类拒绝不再叠加「核心不可达」退化前缀，该前缀仅保留给真实可达性信号（core.reachability）。验证标准：client.ts/notice-layer 单测断言 Unavailable 文案不含前缀且如实呈现 cause
  - 状态：前缀实际源头在后端 `SessionRejection::Unavailable` 的 Display（session_backend.rs:111「核心不可达: {cause}」），前端只透传服务端文本——已改 Display 为「操作不可用: {cause}」并加 Rust 单测；前端半边由 pending-stack 单测钉住（toast 文案携带 cause 且不含「核心不可达」）。

## 2. 会话重命名链路（D3a）

- [x] 2.1 rename-dialog 保存 handler 从 `wa-input` 内部原生 input 显式取值后调用 label API；保存成功以前端收到的 API 结果为准关闭对话框。验证标准：组件级回归测试——渲染对话框、填非空值、保存，断言 fetch 请求体携带该值且成功后关闭
  - 状态：project-rail.ts `renameInputValue()`（原生 input → 宿主 value → 组件状态三级锚点）+ 组件级回归两例（原生 input 取值成功关闭 / 失败留窗内联报错）。
- [x] 2.2 GUI 回归：沙箱内重命名会话后立即查 `GET /api/sessions` 该会话 label 等于输入值（排除静默 no-op）。验证标准：验收脚本或手工 GUI 步骤 + API 断言一致
  - 状态：GUI 路径验证通过——rail 行菜单 → 重命名 → 填 `renamed-by-gui` → 保存，rail 行名实时翻为 `renamed-by-gui`（DOM 抓取确认）。同会话即时消费 label。evidence: [.openspec/round5-evidence/rail-live-label-update.png](/.openspec/round5-evidence/rail-live-label-update.png)。

## 3. label 变更实时广播（D3b）

- [x] 3.1 label 写入成功路径广播既有 session.updated 帧（core 侧写 label 的引擎/通道路径触发既有帧发射点，不扩帧形状）。验证标准：wire 层单测——POST label 后订阅端收到该会话的 session.updated
  - 状态：引擎 `web_set_session_label` 本就 `publish_updated`（五键帧形状不动）；approval_restore_identity_test 扩展钉住「设置/清空均发布 Updated、载荷携带 label、拒绝不发帧」。
- [x] 3.2 rail 行名消费：行收到该会话的 session.updated 且命名来源可能变化时，对该会话做一次轻量投影重取并重渲染行名（按 design 决策 4，不做全列表轮询）。验证标准：前端单测——帧到达触发行名重取与重渲染；GUI 回归——API 写 label 后行名无需刷新即更新
  - 状态：rail `onWsEvent` 对 session.updated/created 触发 400ms 尾沿防抖重取（单会话 detail 带聚焦副作用且无 label，弃用；复用既有 GET /api/sessions）；前端单测断言防抖合并 + 行名重渲染。GUI 半边（API 写 label 行名即时更新）随 5.1。

## 4. P3 打磨

- [x] 4.1 行菜单关闭态对辅助技术隐藏（hidden/inert），a11y 树不再暴露「重命名/归档/移除项目」菜单项。验证标准：domSnapshot/a11y 快照在菜单关闭时不含菜单项
  - 状态：rail 样式表 `wa-dropdown:not([open]) wa-dropdown-item { display: none }`（open 反射属性翻转与 popup 激活同帧）；单测钉住规则存在，快照核对随 5.1 真浏览器。
- [x] 4.2 失败类 toast 增加自动消失（8s 常量，与成功类策略分开）。验证标准：notice-layer 单测断言失败类定时器与时长
  - 状态：notify.ts `ERROR_TOAST_DURATION_MS = 8_000`（error 默认从驻留改 8s 瞬时，显式 duration=0 仍驻留）；notify.test / notice-layer.test 同步更新。
- [x] 4.3 About 实例概览 provider 计数加口径标注（router 侧，含 debug provider），与 Models 注册表区分。验证标准：GUI 截图核对文案
  - 状态：settings-modal About BUILD 段加「router 侧计数，含 debug provider」副注；单测钉住文案，截图核对随 5.1。

## 5. 整体回归与验收

- [x] 5.1 GUI 验收复跑：round5 三个缺陷的最小复现场景逐一消除（排队重排/移除成功、重命名后行名即时更新、API 改 label 行名即时更新）。验证标准：沙箱 GUI 步骤 + 截图证据
  - 状态：完整 GUI 三场景消除——
      • A. 排队管理面（1.4）：18 条入栈后同一柜内点「上移 hold 04」 → API 与 UI 顺序 `03,04,05` → `04,03,05` 一致；点「移除 hold 18」 → API 与 UI 一致缺失项；过程无「核心不可达」误导 toast（notice-layer text=null），与 1.3 的「操作不可用」文案分离。
      • B. 重命名链路（2.2/D3a）：rail 行菜单 → 重命名 → 填 `renamed-by-gui` → 保存，rail 行名实时翻为 `renamed-by-gui`，组件级 renameInputValue 三级锚点未丢输入。
      • C. label 实时广播（3.2）：POST /api/sessions/{key}/label 写入 `live-update-from-api-2`，等 2 秒抓 rail DOM，全树扫描命中 SPAN `live-update-from-api-2`，行名即时翻为新 label，验证 WS session.updated 帧 + 400ms 防抖重取链路。
  - 主 agent GUI 回归负责
- [x] 5.2 `cargo test` 全绿 + `openspec validate` 通过；round3 任务 6.1 追加本轮三次复现证据（行为与本 change 3.x 的帧机制相关，修复仍归 round3）
  - 状态：`CARGO_TARGET_DIR=target-qa cargo test` workspace 全绿（含本轮新增 4 组单测）；`pnpm -C sebas-webui/frontend test` 652/652 绿；`pnpm run build` 通过；`openspec validate fix-webui-qa-defects-round5` 通过。round3 6.1 的 round5 复现证据此前已补记（该任务行内）。
