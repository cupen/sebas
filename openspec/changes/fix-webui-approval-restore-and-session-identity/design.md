# Design: fix-webui-approval-restore-and-session-identity

## Context

第二轮 fake-claude 全链路 GUI 验收确认了六类缺陷的根因（代码走查已定位到行级）：

1. **审批不可恢复**：审批卡（`sebas-review-cards`）唯一数据源是 WS `permission.requested` 一次性推送；tokio broadcast channel 不向新订阅者重放，`SessionInfo` 只带泊车**计数**（`parked_approvals`）不带请求体，刷新后面板无从重建（`sebas-webui/frontend/src/components/review-card.ts:142-176`、`sebas-dispatch/src/engine/acp_events.rs:31-46`、`sebas-webui/src/api.rs:2436-2499`）。
2. **interrupt 半程收尾**：`web_cancel_session` 只发 `AcpCommand::Cancel`；driver 收尾只发 `Finished`；泊车集合只在「批复」或「会话终结」时清除，cancel 不解除；`turn_engaged = Spawning || (Active && (WORKING || parked_count>0))` 因此永真；`Finished` 在 `apply_event` 不 append 任何 transcript 条目（`sebas-dispatch/src/engine/mod.rs:528-535, 1635-1658, 1101-1123`、`sebas-acp/src/claude/driver.rs:713-766`、`sebas-dispatch/src/engine/stall.rs:84-113`）。
3. **归档丢身份**：`ArchiveEntry` 无身份字段；归档路由手里有完整 `SessionInfo` 却只存 4 项；恢复走 `Mapping::dormant()` 全落默认（`sebas-webui/src/archive.rs:21-44`、`sebas-webui/src/api.rs:2120-2200`、`sebas-dispatch/src/state.rs:276-293`）。
4. **项目标题不联动**：主区标题只消费 shell 的 `selectedPath`，唯一写入口是项目行点击（`rail-select` 事件）；会话聚焦链路（`sebas:rail-focus` → summary 刷新）从不反推项目（`sebas-webui/frontend/src/app-shell.ts:84,631-656`、`views/project-rail.ts:499-534`、`views/dashboard.ts:796-798,1125-1159`）。
5. **rail 展开不持久**：`expanded` 是纯内存 `@state`、初始 `{}`（全收起）、无 localStorage 键（`views/project-rail.ts:127,500,864`）。
6. **桩保真**：SDK `ThinkingBlock.signature` 为必填 `String`，fake-claude 的最终 thinking assistant 帧不带 `signature` → 整帧 `MessageParse` → driver 按「未知消息」warn 后丢弃（`cc-agent-sdk` `messages.rs:240-246`、`tests/bin/fake-claude.rs:661-685`、`sebas-acp/src/claude/driver.rs:683-697`）。thinking_delta 流式增量本身可通，不受影响。

## Goals / Non-Goals

**Goals**
- 审批请求获得「读模型 + 推送」双通道；UI 重建与推送按 `request_id` 幂等合并。
- interrupt 变成全程收尾：泊车释放（fail-closed）、transcript 停止条目、`turn_engaged` 复位跨刷新稳定。
- 归档条目携带会话身份，恢复原样带回；旧档如实回退。
- 聚焦会话反投影响项目上下文；rail 展开态持久化。
- 补齐越界禁用原因、会话命名（label）、图标本地化、桩 signature。

**Non-Goals**
- 不改 Feishu 侧审批卡与决策语义（permission-flow 既有决策词汇不变）。
- 不重构 WS 广播通道（broadcast(64) 容量、通知分级不在本期）。
- 不改 cc-agent-sdk（外部 crate）：signature 必填是 SDK 边界，桩侧对齐真实 CLI。
- 不做旧归档条目数据迁移。
- 不引入多标签页/多端状态同步的新语义（读模型按「最后一次拉取」为准）。

## Decisions

### D1 审批恢复：读模型拉取，而非 WS 重放

在 engine 泊车登记处（`acp_events.rs` 已有 parked 集合）暴露枚举：`SessionBackend` seam 新增 `pending_permission_requests(key)`，webui 路由挂 `GET /api/sessions/{key}/approvals`，返回 `[{request_id, tool, args}]`。前端 `review-card` 初始化（sessionKey 就绪时）主动拉取一次，与 WS 推送按 `request_id` 去重合并；批复成功后按 `request_id` 本地摘除 + 拉取校准。

- 备选「连接建立时重放未决事件」：需要 broadcast 层缓存/扫描，多连接语义复杂，且重放的事件流仍解决不了「跨会话打开」时的重建；读模型幂等、可测试、天然覆盖刷新/重连/多标签。推送通道保留不动（实时性不受损）。
- `request_id` 语义沿用 agent-driver 既有命名空间（`claude:tc-N`），不做新键。

### D2 interrupt 收尾：泊车释放挂在 cancel 链路，条目挂在 `Finished` 处理

- 泊车释放：`web_cancel_session`（及会话终结路径）在发 Cancel 后同步清除该会话 parked 集合（复用 `note_permission_resolved` 的底层「解除登记」能力，语义为 fail-closed 释放而非 allow/deny），随后 `parked_count=0` 让 `turn_engaged` 自然回落。
- transcript 条目：在 `apply_event` 的 `Finished` 分支对「被取消」的回合 append 一条错误类条目（文案含「回合被停止」，复用既有错误条目渲染，不新增 entry kind）。driver 的 `Finished` 事件需携带取消原因或在 engine 侧由 cancel 命令打标——采用**engine 侧打标**（cancel 命令置 flag，`Finished` 时消费），避免改 ACP 事件 wire。
- 备选「driver 在 Finished 里带 reason」：改 wire 形状、影响其他 driver；engine 打标改动最小且 cancel 本来就是 engine 发起。
- 已释放请求的迟到批复：批复路由对 `request_id` 未命中时返回 404/409（沿用 typed rejection 风格），不复活任何状态。

### D3 归档身份：条目扩字段 + Dormant 带参

`ArchiveEntry` 增加可选字段 `agent_kind` / `desired_mode` / `current_model` / `available_models`（`Option`，serde default，向后兼容）。归档路由把 `SessionInfo` 四项传入；恢复链路把四项传到 `web_restore_session` → `Mapping::dormant()` 增加身份参数（None 时维持现默认）。wire 上 `agent_kind=None` 的呈现保持 "default agent" 回退（已有行为），前端不为旧档编造身份。

### D4 项目上下文联动：焦点反投影到 shell 的 `selectedPath`

dashboard 在 `sebas:rail-focus` 处理与「新建落地 / 恢复聚焦」路径上，用 `detail/summary.active_session.project_id` 在项目列表中查找对应路径，找到则派发既有 `rail-select` 语义（或直接回调 shell 更新 `selectedPath`）——保持 shell 单一所有权，不另立状态源。找不到（会话项目不在注册列表）时维持「未选择项目」。

- 备选「标题直接读 detail.project」：双源驱动，项目行点击与焦点跟随会互相覆盖且顺序敏感；投影回 `selectedPath` 让两条链路收敛到同一状态。

### D5 rail 展开持久化：localStorage + 聚焦缺省展开

`project-rail` 的 `expanded` 读写 `localStorage`（键 `sebas.rail-expanded`，值为项目路径数组；与 `split-persist.ts` 同风格）。初始化：优先读持久化；某项目无记录且其下有聚焦会话（rail 已知 active key）时缺省展开。写入时机：toggle 时。

### D6 会话命名：label 字段贯穿

存储沿用会话 label 概念（归档条目已有 `label`；会话行状态需要 `label` 进 `SessionInfo`）。`PATCH /api/sessions/{key}/label`（或复用既有 mutation 风格路由）设置/清空；rail 行与对话框命名顺序：label → 首条 prompt 预览 → 短 id。零轮占位可否命名：可（label 是操作者自由输入）。

### D7 桩 signature：对齐真实 CLI

fake-claude 的 thinking assistant 帧补 `signature`（固定假签名即可，真实 CLI 由 `signature_delta` 补齐的最终块必带）。driver 日志：`MessageParse` 的 warn 文案带上subtype/类型上下文并降频（同一会话连续 parse 失败只告警一次），避免生产噪音——不改 SDK。

### D8 图标本地化

前端图标改为本地资源：仅引入 dashboard 实际用到的 FA 子集（SVG 内联或拷贝进 `frontend/public`），构建期打包进 dist；移除 CDN `<link>`/`fetch`。验收：断网（或 hosts 屏蔽 CDN）刷新无 403、无破图。

## Risks / Trade-offs

- [读模型与推送竞态：拉取返回后 WS 才到同一请求] → 前端按 `request_id` 去重；批复后本地摘除 + 下次拉取校准，状态最终一致。
- [cancel 释放泊车与 driver 自身收尾并发] → 释放动作幂等（重复解除无害）；释放只影响 engine 侧登记，不影响 driver 内部 hook 等待（进程随后被断开）。
- [`turn_engaged` 语义变化影响既有测试] → 语义收紧方向是「孤儿泊车不再算 engaged」，既有依赖 parked>0 的用例需逐个核对（预计集中在 turn_stall / tiered-notices）。
- [ArchiveEntry 扩字段的新旧档共存] → 全部 `Option` + serde default；旧档恢复路径保持现默认，不做迁移。
- [rail 展开持久化键与多项目增删] → 按路径存数组，恢复时对不存在路径静默忽略（与项目删除语义一致）。
- [label 与首条 prompt 命名顺序变化] → 只影响「设置了 label」的会话；未设置 label 的行为完全不变。

## Migration Plan

无部署迁移：归档文件向后兼容（旧条目可读），wire 变更均为增量字段/增量路由。回滚 = revert 提交即可，旧归档条目在新代码下依旧可恢复。

## Open Questions

无（SDK signature 行为已由 registry 源码核实；「真实 CLI 最终 thinking 帧必带 signature」沿用 spike wire fixtures 的保真契约，记为假设）。
