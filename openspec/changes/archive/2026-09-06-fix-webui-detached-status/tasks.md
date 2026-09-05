## 1. 拒绝语义（D2 + D3）

- [x] 1.1 `sebas-webui/src/session_backend.rs`：新增 `SessionRejection::BackendUnavailable { backend, cause }`（Display "执行体不可用: {backend} — {cause}"），补 serde 往返单测（新变体序列化/反序列化、既有变体形状不变）
- [x] 1.2 `src/agent_backend.rs`：native 缺凭据的拒绝改投 `BackendUnavailable`；`DualSessionBackend::spawn` 增 backend 提示校验（None/acp/`acp:<kind>`/native 之外 → `BackendUnavailable`，cause 指名未知提示，不建会话）；单测覆盖：未知 hint 拒绝且无会话、缺省仍默认 acp、native 缺凭据文案不再含"核心不可达"

## 2. detached GatewayInfo 装配（D1）

- [x] 2.1 `src/webui_cmd.rs`：用 `GatewayConfig::parse` 装配静态事实（listen/debug/has_auth），替换 `GatewayInfo::default()`；单测覆盖配置含/不含 `[gateway]` 两种输入的装配结果
- [x] 2.2 provider 列表接状态库真源：经 `state_snapshot`（或 backend trait 增只读方法）取 provider 视图填入 `/api/settings` 的 gateway 段；真源不可用时响应带 `providers_available: false`；route 层单测用 fake backend 断言三种情形（有数据/空/不可用）

## 3. 前端透传核对与验收

- [x] 3.1 核对 composer provider 标签逻辑（`workbench-composer.ts`/`dashboard.ts`）对新响应形状的兼容，必要时调整；前端单测覆盖"有 provider / 不可用"两种渲染
- [x] 3.2 双形态沙箱验收：release 构建，in-process 与 detached 各验证——provider 标签一致且反映运行期改名、`backend: warp-drive` 返回 typed rejection 且无会话、native 缺凭据文案指名执行体；`cargo test` 全绿后 conventional commit 提交
