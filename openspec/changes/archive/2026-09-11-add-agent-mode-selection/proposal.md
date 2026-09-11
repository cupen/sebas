# add-agent-mode-selection

## Why

创建会话时操作者无法选择 agent 的权限模式：`POST /api/sessions` 只有 `prompt/project_id/agent/model` 四个字段，前端没有 mode 选择器，mode 在 webui 创建链路上完全不存在。节点链路虽有 `SessionMode`（ask/edit/allow/auto）门控机制，但 webui 创建远端会话时硬编码 `None`（恒为 ask），且无任何 UI 入口；本机 claude 会话则从不设置 `--permission-mode`。操作者想"这次先让它放开手跑"只能去改 agent 配置。同时测试希望对 mode 链路做全流程验证而不消耗真模型 token——现有 fake 桩（fake-claude、内置 test provider）已让 e2e 零 token，但 fake-claude 对 mode 没有任何差异化行为，无法断言。

## What Changes

- 会话创建 wire（`POST /api/sessions`）新增可选 `mode` 字段，词汇复用节点侧 `SessionMode`：`ask / edit / allow / auto`（缺省 = 不发送 = agent 默认行为）；未知值 400 如实拒绝，不静默降级。
- mode 贯通两条放置路径：本机 claude 会话按约定映射为 `--permission-mode`（ask→不传、edit→acceptEdits、allow/auto→bypassPermissions）；远端节点直传已有 `SessionOp::Spawn.mode`（替换硬编码 `None`）。其它 agent（native、通用 ACP）接受该字段但不生效（非致命，同 model 的既有语义）。
- 中途切换：新增 `POST /api/sessions/{key}/mode`（仿 model 端点形状）。本机 claude 走 driver 运行时 `set_permission_mode`，agent 接受与否经事件流反馈（新增 `ModeChanged` 事件；失败非致命，UI 报错误、mode 不变）；远端走已有 `SessionOp::SetMode`。占位会话（0-turn）记住 mode、首条消息 spawn 时应用。
- 修复 claude driver 存活探针缺陷：watchdog 每秒发 `set_permission_mode(Default)` 会覆盖会话配置的 mode——探针改为发会话当前 mode（no-op 语义不变）。
- 前端：创建表单（workbench composer）新增 mode 下拉（默认"agent 默认"）；会话头部展示 mode 并提供切换入口（复用远端会话已有 `mode-tag` 渲染基础）。
- fake 层与测试强化（全零 token）：fake-claude 对 mode 产生可断言的差异化行为（bypassPermissions 下 `perm` 场景不走 hook 直接放行）并在 journal 记录 mode 变化；新增 e2e 用例断言 argv 透传与中途切换；验收套补远端节点 mode 旅程。

## Capabilities

### New Capabilities

（无——本变更全部落在既有 capability 的 requirement 变化上。）

### Modified Capabilities

- `webui`：HTTP route surface 增加 `POST /api/sessions/{key}/mode`（中途模式切换）；`POST /api/sessions` 增加可选 `mode` 字段与词汇校验；Session dashboard 增加 mode 展示/切换；`SessionBackend` seam 方法增加 mode 维度。
- `permission-flow`：会话模式从"仅节点链路的门控事实"扩展为"创建时可选、会话中可切的控制面期望值"——本机 claude 路径的 mode 映射与生效回报纳入同一 desired/effective 词汇。
- `acp-driver`：claude driver 新增启动 `--permission-mode` 应用、运行时 mode 切换命令与 `ModeChanged` 事件；存活探针不再覆盖会话 mode。
- `node-session-channel`：放置请求的 `mode` 字段从"恒为空"变为"由创建请求携带"，中途切换启用 `SessionOp::SetMode`。
- `testsuite-process-e2e`：新增 mode 透传与中途切换的进程级用例（fake-claude journal/行为断言）。
- `testsuite-acceptance`：新增远端节点 mode 旅程（创建带 mode＋中途切换，零 token）。

## Impact

- **wire/API**：`POST /api/sessions` 增字段（向后兼容）、新端点 `POST /api/sessions/{key}/mode`、快照/snapshot 增加 mode 字段。
- **代码**：`sebas-webui/src/api.rs`、`session_backend.rs`、`src/core_channel/{protocol,client,server}.rs`、`sebas-dispatch/src/engine/mod.rs`（`Out::WebSpawn`）、`src/dispatch.rs`（`handle_web_spawn`）、`src/session_boot.rs`、`sebas-acp/src/claude/driver.rs`、`src/node_link/placement.rs`（映射）、前端 `client.ts`、`workbench-composer.ts`、dashboard。
- **测试桩**：`tests/bin/fake-claude.rs`（差异化行为＋journal）；新 e2e/验收用例全部走既有桩，零真 token。
- **兼容性**：无 breaking——不发送 mode 的调用方行为与今日逐字一致。

## Non-goals

- 不暴露 plan 模式（词汇固定为 ask/edit/allow/auto 四档）。
- 不为通用 ACP driver（opencode 等）与 native 内核实现 mode 生效逻辑——接受但不生效，留待后续 change。
- 不改变节点侧 `SessionMode` 门控语义本身（`record_gate` 行为不动）。
- 不动真实 agent 的计费/凭据路径：所有新测试用例不调用真模型。
