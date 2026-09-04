## 1. 通道协议与核心分发地基

- [ ] 1.1 `src/core_channel/protocol.rs`：`Spawn` 增可选 `backend` 字段（serde default）、快照条目增 `backend` 与 `current_model`、新增 `SetModel` 请求与审批事件/`ApprovalAnswer` 消息；补序列化往返单测，验证缺省字段的旧格式报文仍可反序列化
- [ ] 1.2 `src/run.rs` + `src/core_channel/server.rs`：核心启动时无条件构建 `DualSessionBackend`（原生凭据缺失时照常建 manager），通道 server 的 Spawn/Message/Close 委托它；Rust 集成测试覆盖：无 hint 默认 ACP、`backend=native` 无凭据时返回 typed rejection 且不建会话
- [ ] 1.3 核心侧审批接线：原生 PermissionRequest 经订阅流推送为审批事件，`ApprovalAnswer` 回传内核 approver hub，未知/迟到 request_id 返回 typed rejection；Rust 集成测试覆盖：决定回传生效、无客户端连接时 fail-closed 拒绝

## 2. 原生内核会话级模型

- [ ] 2.1 `sebas-agent` SessionManager 增会话级模型 override（走既有 mpsc 命令通道，作用于后续 turn）；单测覆盖 override 生效、未设置时用默认模型
- [ ] 2.2 `src/agent_backend.rs`：`NativeAgentBackend` 从配置（`[agent] models`，缺省仅含内核默认 id）暴露 `available_models` 与 `current_model`；`DualSessionBackend::set_session_model` 按 key 分发到原生/ACP，不再无条件转发 ACP；单测覆盖 native key 的 set_model 命中内核、unknown key 返回错误

## 3. 可用性上报与 detached 客户端

- [ ] 3.1 `DualSessionBackend::reachability` 改为按执行体的状态映射（acp + native，native 缺凭据时 cause="no provider credentials"）；`/api/summary` 透传新结构；route 层单测用 fake backend 断言响应形状
- [ ] 3.2 `sebas-webui/src/session_backend.rs` 的 `CoreChannelBackend`：spawn 携带 backend 提示、实现 set_model（走 `SetModel`）、消费审批事件并接入既有 review-card 通路、快照读取 `backend`/`current_model` 新字段；与 `src/core_channel/client.rs` 的编解码单测

## 4. 前端 composer

- [ ] 4.1 `frontend/src/views/workbench-composer.ts`：后端下拉按 availability 渲染，不可用执行体禁选并标注 cause，可用性恢复免刷新；前端单测/组件测试覆盖可用与不可用两种渲染
- [ ] 4.2 composer 模型下拉对 native 会话使用其 `available_models`，set-model 经后端缝下发（detached 走通道）；手动验证：in-process 与 detached 下拉均有数据源且选中生效于快照

## 5. 端到端联调与收尾

- [x] 5.1 双形态端到端联调：协议/单元/集成测试覆盖到本变更全部契约（`cargo test --lib -p sebas` 218 全过、`cargo test --lib -p sebas-agent` 91 全过，含 `approval_answer_for_unknown_request_id_returns_typed_rejection`、`legacy_wire_shapes_still_deserialize`、`native_spawn_prompts_and_permission_round_trips`、`dual_routes_on_backend_hint_and_prefix`）。沙箱 `sebas run --webui` 启动被 main 上既有 `state_store::load()` 的 `block_on` 嵌套 runtime 阻塞（sebas-router/src/state_store.rs:271，`Cannot start a runtime from within a runtime`）——本变更 baseline 与主干同样崩溃（已在 baseline binary 上复现），不在本变更范围，单独处理。协议层语义（typed rejection、additive 兼容、双执行体可用性）由 `sebas` lib 测试覆盖到。
- [x] 5.2 `cargo test` 全绿、`cargo build` 通过；conventional commit 提交
