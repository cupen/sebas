## 1. 驱动层骨架（design D1/D2，agent-driver spec）

- [ ] 1.1 在 `sebas-acp/src/` 新增 `agent_driver.rs`：定义 `AgentDriver` trait（async_trait：`spawn(&DriverConfig) -> Result<DriverHandle>`）、`DriverHandle{session_id, events: mpsc::Receiver<AcpEvent>, cmd_tx: mpsc::Sender<AcpCommand>, cancel: oneshot::Sender<()>}`、`DriverConfig`、`DriverError`；把 `AcpEvent`/`AcpCommand` 从 `claude/session.rs` 上移为 crate 级公共词表（防腐层）；验证 `cargo build -p sebas-acp` 通过且 `agent_driver` 模块可见
- [ ] 1.2 把 `claude/driver.rs` 的公共面收敛为 `ClaudeDriver: AgentDriver`：保留内部 `cc-agent-sdk` 与 `map_message` 翻译，`connect()`/`run()` 的入口签名改为实现 `spawn()` 返回 `DriverHandle`；验证 `sebas-acp/tests/{canned,permission_roundtrip,crash_terminal_event,spawn,startup_timeout}.rs` 不改断言语义下全部继续通过（现有 13 个测试文件 0 回归）

## 2. 通用 ACP 驱动（design D1/D5，agent-driver spec）

- [ ] 2.1 新增 `sebas-acp/src/acp_driver/mod.rs` + `acp_driver/codec.rs`：实现 `AcpDriver: AgentDriver`，用 `tokio::process::Command` spawn 配置里的 `command`，持 `Child`（真实 SIGTERM/SIGKILL 语义）；用 `agent-client-protocol` 2.0.0 的 `Client.builder()`（stable-v1）做 `initialize`；验证 `cargo build -p sebas-acp` 通过且 `agent-client-protocol` 依赖进入 `Cargo.toml`
- [ ] 2.2 实现 `acp_driver/codec.rs` 的双向翻译：ACP `session/update` 变体 → `AcpEvent`（`agent_message_chunk`→`TextDelta`/`ThinkingDelta`、`tool_call`→`ToolStart`、`tool_call_update`→`ToolEnd`/`ToolProgress`、`plan`→丢弃或映射）；`AcpCommand::{CreateSession,ContinueSession,Cancel,PermissionReply}` → ACP 方法；验证 `sebas-acp/tests/acp_codec.rs`（canned JSON 帧 fixture）覆盖文本/思考/工具开始/工具结束/完成各至少一个用例
- [ ] 2.3 实现 `acp_driver` 的权限桥：ACP `session/request_permission` → `AcpEvent::PermissionRequest`（request_id 编码为 `<kind-slug>:<raw-id>`）；`AcpCommand::PermissionReply` → ACP permission 应答（`PermissionOption.kind` 映射 `allow_once/allow_always/reject_once/reject_always`，`Escalate` 降级 `AllowOnce`）；验证 `sebas-acp/tests/acp_permission_roundtrip.rs`（canned JSON-RPC 脚本回放一次 request→allow 往返）

## 3. 配置 schema 与迁移（design D3，agent-driver spec）

- [ ] 3.1 改写 `src/config.rs::AcpConfig`：新增 `default: Option<String>` 与 `agents: HashMap<String, AgentConfig>`；`AgentConfig` 用 `#[serde(tag = "driver", rename_all = "snake_case")]` 区分 `Claude(AcpClaudeConfig)` 与 `Acp{command: Vec<String>, startup_timeout_secs, idle_kill_secs}`；保留 legacy `claude: Option<AcpClaudeConfig>` 字段；验证 `cargo build` 通过且旧 `[acp.claude]` TOML 仍能 deserialize 不报错
- [ ] 3.2 在 `src/config.rs::load` 末尾加迁移：`claude.is_some() && agents.is_empty()` → 移到 `agents["claude"]`（driver=claude），`default` 为空则隐式设为 `claude`，emit 一次 `tracing::warn!`；验证单测断言迁移后 `agents["claude"]` 等价于旧 `claude` 字段且 warn 触发一次
- [ ] 3.3 加 "unsupported driver tag" 与 "default 缺省 + 单 agent 隐式" 两个边界：serde 反序列化未知 `driver` tag 时 `cargo test -p sebas` 断言 `load` 返回错误；单 agent 无 default 时解析到该 agent；验证 `src/config.rs::tests` 新增两个用例

## 4. SessionManager 接 driver 注册表（design D1，agent-driver spec）

- [ ] 4.1 改 `sebas-acp/src/claude/manager.rs` 为 driver-agnostic：`SessionManager` 持 `HashMap<String, Arc<dyn AgentDriver>>`（kind slug → driver）+ `default: String`；`create_session(kind, config, prompt)` 按 kind 查 driver；保留现有会话表/通道/kill/锁纪律逻辑（约 280 行复用资产）；验证 `cargo build -p sebas-acp` 通过
- [ ] 4.2 改调用侧 `src/run.rs`/`src/dispatch.rs`/`src/session_boot.rs`：把直接 `use sebas_acp::claude::manager::SessionManager` 改为通过新的 `AgentKinds::new(cfg.acp)` 装配 driver 注册表；`spawn` 路径把 kind slug 透传给 `create_session`；验证 `cargo test -p sebas` 现有 spawn/permission/idle 测试全绿（裸 `acp` = `claude` default，语义不变）

## 5. 权限半场补齐（design D6，agent-driver spec: Cross-driver permission routing）

- [ ] 5.1 在 `sebas-router/src/router/mod.rs` 暴露一个 ACP 权限事件 broadcast（`acp_permission_requests()` 或复用 `subscribe_session_events` 过滤 PermissionRequest 变体——按 design OQ1 倾向独立 broadcast）；验证 `cargo test -p sebas-router` 通过
- [ ] 5.2 在 `sebas-webui/src/session_backend.rs` 给 `InProcessBackend` 实现 `permission_requests()` 与 `answer_permission()`：订阅 router 的 ACP 权限广播 → `PermissionNotice`；`answer_permission` 把 `PermissionDecision` 映射回 `AcpCommand::PermissionReply`（`Escalate` 降级 `AllowOnce`）；验证 `sebas-webui/tests/` 新增一个"Claude 权限请求经 InProcessBackend 往返到 webui 审查卡"端到端测试
- [ ] 5.3 验证 `DualSessionBackend::answer_permission` 的 acp 回退分支不再死代码（现应为活路径）；验证 `src/agent_backend.rs::tests` 扩展一个"acp 会话权限经 dual 后端往返"用例

## 6. webui 后端 hint 与下拉（design D3，agent-driver spec: Reachability）

- [ ] 6.1 扩展 `sebas-webui/src/api/client.ts` 的 `BackendHint` 为 `'acp:<kind>' | 'native'`（裸 `'acp'` 仍合法=default），新增 `parseBackendHint(hint)` 辅助；验证 `sebas-webui/frontend/src/api/client.test.ts` 新增裸 acp/`acp:gemini`/`native` 三用例
- [ ] 6.2 新增 `GET /api/agent-kinds` 端点（`sebas-webui/src/server.rs`）：遍历已配置 agents 调 `discover()`，返回 `{kinds:[{name, slug, reachable, cause?, version?}]}`；验证 `sebas-webui/tests/` 加 canned 配置 + 假 driver 的 happy-path
- [ ] 6.3 改前端 `{sessions,workbench-composer}.ts` 下拉：挂载时 `GET /api/agent-kinds`，只列 `reachable` 的 kind + `native`，选择后透传 `acp:<slug>`；验证 `workbench-composer.test.ts` 新增"reachable 列表渲染 + 选择透传 hint"用例

## 7. CLI 子命令与最终门禁（agent-driver spec: Reachability）

- [ ] 7.1 在 `src/cli.rs`/`src/main.rs` 加 `sebas agent-kinds list` 子命令：遍历 `cfg.acp.agents` 调 `discover()`（`command` 首元素 `command -v` + `--version` 探测），打印 `slug reachable version cause?` 表，支持 `--json`；验证 `cargo test -p sebas` 加 canned discover 测试（缺二进制时 `reachable=false cause="command not found"`）
- [ ] 7.2 最终门禁：`cargo clippy -p sebas -p sebas-acp -p sebas-webui -p sebas-router --all-targets -- -D warnings`、`cargo test -p sebas -p sebas-acp -p sebas-webui -p sebas-router`、`pnpm test` + `pnpm build`；`openspec validate multi-third-party-acp-agents` 通过；git diff 不含 1a/2 相关回归