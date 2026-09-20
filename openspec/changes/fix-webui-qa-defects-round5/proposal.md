## Why

第五轮 WebUI 全链路黑盒 GUI 验收（真实浏览器 + fake-claude 沙箱，round4 修复之上）新发现 2 个 P1 缺陷与 3 个 P3 打磨项：裸 core 内嵌部署（标准 `sebas core --webui` 形态）下，待执行队列的重排/移除 API 恒 503 且错误文案误导（真实原因是复合后端未转发，却提示「核心不可达」）；会话重命名对话框保存静默失败（API 直接写 label 正常，GUI 输入值丢失）；label 变更不产生会话更新帧，rail 行名必须手动刷新才更新。三者破坏 workbench-turn-queue 与会话命名的既有契约，须在功能继续叠加前修复。

## What Changes

- **复合后端 pending 管理面转发（P1，实现修复 + spec 澄清）**——`DualSessionBackend` 按 key 把 `pending` / `remove_pending` / `move_pending` 路由到承载侧（内嵌形态即 acp 桥），类型化拒绝（Unknown / AlreadyStarted / PriorityConflict / OutOfRange）原样透传；WebUI 不再把后端路由缺口渲染成「核心不可达」前缀的误导文案。
- **会话重命名链路修复（P1，实现修复）**——rail 重命名对话框保存把输入值真实写入 label API（当前输入值在保存链路丢失，静默 no-op）；后端 label API 已验证可用，仅前端修复。
- **label 变更实时广播（P1，spec 澄清）**——label 写入路径触发会话更新帧，rail 行名消费该帧实时更新，无需整页刷新（对齐首条消息预览的既有实时性要求）。
- **P3 批量打磨（无 spec 变化）**：关闭状态的行菜单项对辅助技术隐藏（a11y 树不再暴露）；失败类 toast 增加自动消失策略；About 实例概览的 provider 计数与 Models 区注册表口径加区分标注。

## Non-goals

- 不修 round3 已跟踪任务：6.1（聚焦会话未读徽标误闪，本轮验收已补充三次复现证据并回写该任务）、6.2、4.2/6.3——归 round3 收口。
- 不改归档/恢复 GUI 交互（后端语义已由验收套件覆盖，本轮仅记录 GUI 自动化受阻）。
- 不动 pending 队列数据结构与 wire 协议（session.updated 五键帧形状不变，label 走既有帧的载荷扩展而非新帧型）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `core-session-channel`：pending 管理需求补场景——任何部署形态（含内嵌复合后端）下 pending 管理操作都必须可达，类型化拒绝原样透传，不得以笼统「不可用」替代。
- `project-session-actions`：会话命名需求补场景——操作者设置/变更 label 后，rail 行名经会话更新事件实时刷新，无需页面重载。

## Impact

- 后端：`src/agent_backend.rs`（DualSessionBackend 三个方法转发）、`sebas-webui/src/session_backend.rs` 与 `api.rs`（错误映射核查，503→按真实原因类型化）
- 前端：rename-dialog 保存链路取值、rail 行名对会话帧的消费、notice-layer 自动消失、行菜单关闭态 a11y、About 口径标注
- 测试：DualSessionBackend 转发单测、label 变更触发帧的 wire 断言、GUI 回归（重命名/排队重排路径）
