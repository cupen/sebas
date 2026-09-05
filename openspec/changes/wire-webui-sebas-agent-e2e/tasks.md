## 1. 通道协议与核心分发地基

- [x] 1.1 `src/core_channel/protocol.rs`：`Spawn` 增可选 `backend` 字段（serde default）、快照条目增 `backend` 与 `current_model`、新增 `SetModel` 请求与审批事件/`ApprovalAnswer` 消息；补序列化往返单测，验证缺省字段的旧格式报文仍可反序列化
  （证据：`Spawn.backend`/`CreatePlaceholder.backend`/`ApprovalAnswer` 均带 `#[serde(default)]`；`SessionInfo` 增 `#[serde(default)] backend`（sebas-router/src/router/events.rs，D4：由复合后端打标 acp/native）；单测 `every_request_and_response_variant_round_trips`、`stream_frame_parses_back_in_order`、`legacy_wire_shapes_still_deserialize`（含旧格式 SessionInfo 反序列化断言））
- [x] 1.2 `src/run.rs` + `src/core_channel/server.rs`：核心启动时无条件构建 `DualSessionBackend`（原生凭据缺失时照常建 manager），通道 server 的 Spawn/Message/Close 委托它；Rust 集成测试覆盖：无 hint 默认 ACP、`backend=native` 无凭据时返回 typed rejection 且不建会话
  （证据：run.rs 无条件装配 `DualSessionBackend`（webui 与通道 server 共享）；`dispatch` 的 Spawn/CreatePlaceholder/SetSessionModel/Message/Close 全部委托 backend；测试 `dual_routes_on_backend_hint_and_prefix`、`native_missing_credentials_rejection_names_the_backend`、`unknown_backend_hint_rejects_without_session`（agent_backend.rs））
- [x] 1.3 核心侧审批接线：原生 PermissionRequest 经订阅流推送为审批事件，`ApprovalAnswer` 回传内核 approver hub，未知/迟到 request_id 返回 typed rejection；Rust 集成测试覆盖：决定回传生效、无客户端连接时 fail-closed 拒绝
  （证据：server.rs `serve_subscription` 交错推送 `ApprovalRequested` 帧、`dispatch` 的 `ApprovalAnswer` → `backend.answer_permission` → 内核 ApproverHub；测试：决定回传 `native_spawn_prompts_and_permission_round_trips`、未知/迟到 request_id typed rejection `approval_answer_for_unknown_request_id_returns_typed_rejection`（core_channel/tests.rs）、无应答 fail-closed `ask_without_approver_fails_closed`（sebas-agent loop_，内核既有路径））

## 2. 原生内核会话级模型

- [x] 2.1 `sebas-agent` SessionManager 增会话级模型 override（走既有 mpsc 命令通道，作用于后续 turn）；单测覆盖 override 生效、未设置时用默认模型
  （证据：`SessionCmd::SetModel` + `SessionHandle::set_model`；空闲期直接生效，turn 中先记 `pending_model`、turn 结束后应用（"下一次 turn 生效"不被吞掉）；单测 `set_model_override_applies_to_next_turn_and_unset_falls_back_to_default`）
- [x] 2.2 `src/agent_backend.rs`：`NativeAgentBackend` 从配置（`[agent] models`，缺省仅含内核默认 id）暴露 `available_models` 与 `current_model`；`DualSessionBackend::set_session_model` 按 key 分发到原生/ACP，不再无条件转发 ACP；单测覆盖 native key 的 set_model 命中内核、unknown key 返回错误
  （实现修正：配置源按 D5 走全 env —— `SEBAS_AGENT_MODELS`/`SEBAS_AGENT_MODEL`，与内核既有配置面一致；`[agent] models` 文件配置面另立 change。证据：`build_native_manager` 推导模型清单、`NativeSession.info()` 透出 `available_models`/`current_model`；`DualSessionBackend::set_session_model`/`spawn_with` 按 key/hint 分发；单测 `dual_set_session_model_routes_native_key_and_rejects_unknown`、`dual_routes_on_backend_hint_and_prefix`（含 spawn 期模型生效于快照））

## 3. 可用性上报与 detached 客户端

- [x] 3.1 `DualSessionBackend::reachability` 改为按执行体的状态映射（acp + native，native 缺凭据时 cause="no provider credentials"）；`/api/summary` 透传新结构；route 层单测用 fake backend 断言响应形状
  （实现形态：整体 reachability 保持 session authority 门禁（不因 native 缺凭据误伤 acp），逐体可用性经 `execution_bodies()`（acp+native）上报 —— 与 fix-webui-detached-status 的语义收敛；native 的 cause 透传装配期真实原因（"native backend needs SEBAS_AGENT_PROVIDER_API_KEY …"，比字面串更具体）。证据：api.rs summary 带 `execution_bodies` 段；route 层单测 `summary_passes_through_per_body_availability`、`summary_reports_null_bodies_when_backend_does_not_distinguish`（server.rs，FakeBackend 增 `set_execution_bodies`））
- [x] 3.2 `sebas-webui/src/session_backend.rs` 的 `CoreChannelBackend`：spawn 携带 backend 提示、实现 set_model（走 `SetModel`）、消费审批事件并接入既有 review-card 通路、快照读取 `backend`/`current_model` 新字段；与 `src/core_channel/client.rs` 的编解码单测
  （注：`CoreChannelBackend` 实际位于 src/core_channel/client.rs；快照新字段经共享 wire 类型 `SessionInfo`（+SessionRow/session_summary 透传 `backend`）自动携带。证据：client.rs `spawn_with` 携带 backend、`set_session_model` 走 `SetSessionModel`、`ApprovalRequested` 帧 → notices feed、`answer_permission` 走 `ApprovalAnswer`；编解码往返由 protocol.rs 单测 + core_channel/tests.rs 对真实 server 的 CoreChannelBackend 集成测试覆盖）

## 4. 前端 composer

- [x] 4.1 `frontend/src/views/workbench-composer.ts`：后端下拉按 availability 渲染，不可用执行体禁选并标注 cause，可用性恢复免刷新；前端单测/组件测试覆盖可用与不可用两种渲染
  （证据：`nativeAvailability` 随 5s reachability 轮询刷新，native 选项 disabled + cause；单测 `renders the native option disabled with its cause…`、`keeps the native option selectable…`、`re-enables the native option on the next poll without remounting`（workbench-composer.test.ts））
- [x] 4.2 composer 模型下拉对 native 会话使用其 `available_models`，set-model 经后端缝下发（detached 走通道）；手动验证：in-process 与 detached 下拉均有数据源且选中生效于快照
  （证据：跟随模式用聚焦会话 `available_models`（native 会话由 `NativeSession.info()` 透出），切换经 `api.setSessionModel` → `/api/sessions/{key}/model` → 后端缝（in-process 双后端分发 / detached 通道 `SetSessionModel`）；创建期选定模型对 native 也生效（`DualSessionBackend::spawn_with`/`create_placeholder` 应用会话级 override）。e2e 由主代理验收）
