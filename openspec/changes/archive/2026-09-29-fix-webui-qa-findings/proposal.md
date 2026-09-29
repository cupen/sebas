# fix-webui-qa-findings

## Why

2026-09-27 三轮 WebGUI 黑盒验收（主功能轮 42 点 / 认证 RBAC 轮 13 点 / 新构建增量轮 9 点）确认核心链路健壮，但发现 8 项缺陷与 7 项可用性缺失：含未读徽标回归（违反现行 spec `session-unread-badge`）、模式切换契约条目从不落转写、崩溃后会话僵尸 Queued 约 10 分钟等。全部立项修复，避免验收结论只留报告不落行为。

## What Changes

缺陷（QA 编号保留溯源）：
- **DD3+M3（P1，回归）**：非聚焦会话收到回复后侧栏未读徽标不出现、回到会话无未读分界线——源码渲染与 spec 均在，运行时不生效；根因修复 + e2e。
- **D2（P2）**：权限模式切换后转写从不出现 `permission_mode_result` 契约条目（前端已有该条目类型渲染）。
- **D3（P2）**：`停止回复` 在 agent 静默期不即时生效也无反馈，「回合被停止」迟到 60s。
- **D4（P2）**：注册/移除项目后主面板残留先前项目会话视图，侧栏与主面板状态不一致。
- **D5（P2）**：agent 进程崩溃后新消息使会话卡 `Queued` 约 600s，watchdog 强制收尾才自愈。
- **D1（P2，间歇）**：审批挂起期间排队消息自动执行后回复文本重复（旧构建 API 层实证 delta 翻倍；新构建未复现）——先根因调查再修。
- **DD1/DD2（P3）**：侧栏会话行标题不随回合即时刷新；未命名会话标题跟随最新消息而非承诺的「首条消息预览」（口径不改，改实现）。

缺失/可用性（均 P3）：
- **M1**：审批决策后 tool_result 双重折叠、无「已执行/已拒绝」可见反馈。
- **M2**：`/compact` 提交后无任何回执。
- **M6**：四级通知层全程 GUI 不可达——审计接线并补关键操作反馈。
- **M7**：config.toml 的 provider 不出现在 Settings→Models，与 store 两来源关系无解释。
- **M8**：会话重命名后工作台头部仍显示 chat_id 前缀。
- **M9**：About 的 Rust toolchain 恒「未知」（Build 信息段归 add-about-build-info，此处只修探测）。
- **OB2**：root 可对自己行降权/禁用/删除（自锁风险）——前端禁用 + 服务端拒绝双闸。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `session-unread-badge`：徽标呈现条件与分界线回归修复——非聚焦到达必现徽标、回到会话必现分界线。
- `permission-flow`：模式切换 SHALL 产出持久化契约条目并在转写可见；决策结果（执行/拒绝）需有可见反馈。
- `session-lifecycle`：取消请求即时确认呈现；子进程崩溃后的会话 SHALL 快速进入明确终态而非僵尸 Queued。
- `session-slash-commands`：`/compact` 等命令提交 SHALL 有可见回执。
- `project-session-actions`：命名链回归细则——预览口径锚定首条消息（不随最新消息漂移）、回合产生的命名变化即时上行、重命名后聚焦头部即时一致（依赖未归档 change `add-agent-settings-and-session-titles` 的同名需求，见 design）。
- `webui`：项目切换/移除后焦点一致性；关键操作通知反馈；Models 页 provider 来源解释；About toolchain 探测。
- `webui-user-management`：root 对自身行的危险操作防护（前端禁用 + 服务端拒绝）。

## Impact

- 前端：`sebas-webui/frontend/src/views/`（project-rail、transcript-view、workbench-composer、dashboard、settings-modal、notify 接线）。
- 后端：`sebas-dispatch`（崩溃终态、取消确认、mode 契约条目落盘）、`sebas-webui/src`（事件投影）、`sebas-webui` 用户管理 API（自锁拒绝）。
- 测试：新增单测 + webui e2e（DD3 徽标、D2 契约条目、D5 崩溃终态走 Playwright/进程级套件）。

## Non-goals

- 登录「记住我」/活动会话管理面（MA2，Info 级备忘）。
- 侧栏切换会话改写 URL（M5，focused-key 指针驱动是现行设计）。
- 「New session」按钮文案（M4 撤案：词汇表误差，非缺陷）。
- 用户名截断省略（OB1，标准 ellipsis 外观）。
- About 的 Build 信息段扩展（归在途 change `add-about-build-info`）。
