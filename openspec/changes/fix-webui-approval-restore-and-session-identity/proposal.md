# fix-webui-approval-restore-and-session-identity

## Why

对 sebas WebUI 做的第二轮全链路黑盒 GUI 验收（fake-claude 桩驱动，覆盖项目注册、会话创建、消息收发、thinking / tool use / 正文渲染、ask/edit/allow/auto 四档权限模式、审批 allow/deny、模型与 slash 命令、归档恢复、Skills / Settings、CJK 与超长边界、flood 压力；证据见沙箱截图与 core 日志）发现：**挂起的权限审批在页面刷新后不可恢复且无任何 GUI 补救路径**（会话卡死，只能停止回复）、**「停止回复」后 UI 在飞状态跨刷新驻留且 transcript 无停止条目**、**归档恢复丢失 agent 身份与模型目录**（恢复后 agent 显示 "default agent"，后续对话会用错 agent）。另有一批 rail 上下文 / UX / 桩保真缺陷。

## What Changes

- **挂起审批可恢复（高）**：核心侧新增「按会话列出当前待批请求」的读模型（request_id、tool、args、模式）；WebUI 打开/刷新会话时主动拉取并重建 Permission review 面板，与既有 WS 推送按 `request_id` 幂等合并。现状：审批卡是纯一次性 WS 事件驱动的易失 UI，刷新即丢且无任何恢复路径。
- **interrupt 全程收尾（高）**：停止回复时 ① 泊车审批随 cancel 释放（fail-closed：未决请求不再阻塞、`turn_engaged` 回落 false）；② transcript 追加一条「回合被停止」条目（现状：回合无声消失）；③ 停止按钮状态跨刷新稳定复位。
- **归档/恢复保留会话身份（高）**：归档条目扩存 `agent_kind` / `desired_mode` / `current_model` / `available_models`，恢复重建时带入；旧归档条目缺字段时如实回退显示。现状：恢复后 agent 显示 "default agent"、模型目录重置。
- **聚焦会话联动项目上下文（中）**：工作台聚焦会话（rail 点击 / 新建落地 / 恢复聚焦）时，主区项目标题与项目上下文立即跟随该会话所属项目；点击项目行仍可独立选择项目。现状：标题只认「点击项目行」，聚焦会话后仍显示「未选择项目」。
- **rail 展开状态稳定可预期（中）**：项目手风琴展开态持久化（localStorage，按项目路径）；缺省规则：聚焦会话所在项目展开。现状：展开态是纯内存 `@state` 且缺省全收起，刷新即折叠、行为不可预期。
- **越界路径注册给出禁用原因（低）**：Add project 手填路径被拒时（越界 / 不存在）在路径框下显示具体原因，按钮保持禁用。现状：仅禁用无解释（上轮 change 声称的该 UX 修复实测未生效）。
- **会话命名（功能缺失，低）**：rail / 会话头支持重命名（设置 label），显示时 label 优先于首条 prompt 推导名；归档条目沿用既有 label 字段。
- **打磨（P3）**：模型选择菜单标识当前模型；composer 权限模式下拉弹层不再溢出卡片边界。
- **测试桩保真（低）**：fake-claude 的最终 thinking assistant 帧补 `signature` 字段（对齐真实 CLI；现状被 SDK `ThinkingBlock.signature` 必填校验整帧丢弃，core 日志刷 `missing field signature`）；驱动侧日志降噪。
- **WebUI 资源本地化（低）**：前端图标不再从外部 CDN（ka-f.fontawesome.com）加载，打包进 dist——离线 / 受限网络下 403 与破图消除。
- **回归债一并补齐**：上轮 change（fix-webui-qa-defects）遗留的 10.2 沙箱复检、10.3 e2e 套件任务并入本 change tasks。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `permission-flow`: 新增「待批请求读模型与恢复」需求——当前泊车审批可按会话枚举（含 request_id / tool / args），WebUI 重连、刷新后可重建审批面；新增「cancel 释放泊车审批」需求——interrupt/取消时未决审批 fail-closed 解除，`turn_engaged` 不再被孤儿泊车钉住。
- `agent-workbench`: 新增「聚焦会话驱动项目上下文」场景（主区标题跟随）；新增「停止回复全程收尾」需求（transcript 停止条目 + 停止控件跨刷新复位）；新增「rail 展开状态稳定」需求（持久化 + 聚焦项目缺省展开）；「Model selection」需求补充当前模型标识场景；「Parked remote approvals surface in the workbench」需求补充刷新重建场景。
- `project-session-actions`: 「Session archive」需求补充归档条目携带会话身份、恢复重建保留身份的场景；「Session rows are named by the first prompt」需求补充 label 优先与重命名。
- `workspace-root`: 「范围判定是规范化且 fail-closed 的」需求补充拒绝呈现场景——注册被拒时给出具体原因（越界 / 不存在）。

## Impact

- 后端：`sebas-dispatch/src/engine/`（泊车审批枚举读模型、cancel 释放泊车、interrupt 收尾条目、restore 带身份）、`sebas-webui/src/api.rs`（待批请求列表路由、restore 传参）、`sebas-webui/src/archive.rs`（条目扩字段）、`sebas-dispatch/src/state.rs`（Dormant 重建参数）。
- 前端：`sebas-webui/frontend/src/views/`（dashboard 项目标题联动与停止复位、project-rail 展开持久化与 label、workbench-composer 模式弹层与审批重建合并、review-card 读模型拉取与 request_id 幂等）、图标本地化。
- 测试桩：`tests/bin/fake-claude.rs`（thinking 帧补 signature）。
- 既有测试面：`tests/testsuite-webui/tests/`（本组缺陷的 Playwright 回归）、`tests/testsuite_e2e_test.rs` / 验收套件（上轮 10.3 债）。
- 兼容性：旧归档条目（无身份字段）恢复回退现状显示，不做数据迁移。
