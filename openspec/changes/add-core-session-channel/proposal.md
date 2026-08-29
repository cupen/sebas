## Why

生产路径下的 WebUI 是 watchdog 拉起的**独立进程**（`src/webui_cmd.rs`），它做了两件互相叠加的事：一次性 `restore_session_map` 从 state 文件读快照（`webui_cmd.rs:82`），以及丢弃 outbound 接收端（`webui_cmd.rs:93`）。而 session state 文件只在 core **优雅退出时**写一次（`src/run.rs:311`）。

两者相乘的结果：core 正在运行期间，WebUI 展示的是**上一次 core 退出时**的会话列表，并且在整个 core 生命周期内不再变化。它不是"更新慢"，是根本没有事件源——`/events` SSE 即便接上也无内容可推；composer 发出的消息只改本进程内存，不 spawn ACP、不发飞书（现 spec 的 "Standalone detached semantics" 正是在描述这个状态）。

唯一能驱动的 `run --webui` 共享 router 路径，在 spec 中被标记为 legacy，且 ownership guard 禁止它与 watchdog 路径共存。把功能寄托在它上面不构成可交付方案。

## What Changes

- **core 侧新增会话通道**：Unix socket（`~/.sebas/core.sock`，0600），换行分隔 JSON，复用 control RPC 的密钥 + peer uid 双重校验姿态。方法：会话快照、事件订阅、spawn（prompt + project_dir）、发消息、关闭、取回某会话的 turn 内容。
- **router 新增会话事件广播**：每次映射变更向 `broadcast` 发布，作为通道与 SSE 的唯一事件源。core 是唯一状态所有者与唯一 ACP spawn 者，这一点不变。
- **WebUI 引入 `SessionBackend` trait**（对照现有 `AdminAdapter` 的形状）：`routes.rs` 不再直接持 `RouterHandle`，改为持 backend。
- **两个实现**：`run --webui` 用进程内直连实现（行为与今天一致）；`sebas webui` 用 socket 客户端实现。
- **诚实降级**：socket 不可达时，页面明说"core 未连接"，列表给出原因而非空白，composer 禁用并显示真实成因——不再出现"报成功但消息被丢弃"。
- 替换 `webui` spec 的 "Standalone detached semantics"。

## Non-goals

- 不做非 loopback 访问、不做多用户鉴权模型。
- 不改 session state 文件格式与持久化时机。
- 不让 WebUI 成为 ACP 子进程的宿主。
- 不改飞书侧的路由与卡片行为。
- 不引入构建步骤、npm 或前端框架。
- 不在本 change 内做项目化 UI（由 `add-project-workbench` 承担）。

## Capabilities

### New Capabilities
- `core-session-channel`: core 与外部只读/可驱动客户端之间的会话通道——传输与鉴权、方法集、事件语义、单一状态所有权、不可达时的降级契约。

### Modified Capabilities
- `webui`: 独立进程语义从"本地假动作"改为"core 的客户端"；会话数据与驱动能力经由通道，不再依赖启动时的 state 文件快照。

## Impact

- 新增 `src/core_channel/`（服务端）与 `webui_cmd.rs` 侧客户端实现。
- `router/src/router/mod.rs` + `state.rs`：新增会话事件广播。
- `webui/src/`：`SessionBackend` trait；`server.rs`、`routes.rs`、`sse.rs` 由持 `RouterHandle` 改为持 backend。
- `src/run.rs`：启动通道服务端；`run --webui` 传入进程内 backend。
- `webui/tests/session_endpoints_test.rs`：改用 fake backend。
- 无新增外部依赖（tokio UnixListener + serde_json 已在用）。
