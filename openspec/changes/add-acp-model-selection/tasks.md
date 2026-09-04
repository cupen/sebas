# Tasks: add-acp-model-selection

依赖前置：`add-acp-session-id-mapping` 的握手信号扩展（`(routing_id, resumed, acp_session_id)`）优先落地（本 change 的模型信息在其上加 `AcpModelInfo`，避免同信号双改冲突）。若映射 change 未归档，本 change 在最新主干上扩展握手信号亦可。
明确决策：模型列表来自 agent 的 `configOptions`（不硬编码）；`SetModel` 走 AcpCommand 命令通道；create-with-model 在会话建立后应用。
**字段衔接（review-add-state-store R1）**：`current_model`（session 记录字段）是 `add-state-store` SQLite sessions 表同名列的来源；本 change 先落（内存/MappingDto 层），state-store 后收编为其建表列。

## 1. 驱动层：configOptions 上抛 + SetModel 命令

- [x] 1.1 `AcpDriver` 解析 `session/new`/load 响应的 `configOptions`：取 `id=="model"`（或 category==model）的 select 选项 → `AcpModelInfo { current, options }`；握手信号扩展为 `(routing_id, resumed, acp_session_id, Option<AcpModelInfo>)`（与 `acp_session_id` 一并，不破坏 add-acp-session-id-mapping 已定型的前三元组）；无 model 选项 → `None`。验证：`cargo test -p sebas-acp` 全绿（53 项）；mock agent 返回 model 选项时 `SpawnOutcome.model`/`get_model_info` 携带 current+列表（`spawn_outcome_carries_model_info_from_config_options`），无 configOptions 时 `None`（`spawn_without_model_option_reports_none`）。
- [x] 1.2 新增 `AcpCommand::SetModel { session_id, model_id }`；driver 主循环匹配，按**真实 ACP 会话 id**（mapping change 联动）发 `SetSessionConfigOptionRequest::new(acp_session_id, "model", model_id)`；失败映射显式错误（无效模型/agent 无能力），成功更新本地 current + 发 `AcpEvent::ModelChanged`。验证：mock 场景 SetModel 成功→`ModelChanged`（current 更新，wire journal 断言 session_id/config_id/value）；无效 model 被 agent 拒绝→非 terminal `Error`、current 不变。
- [x] 1.3 mock 夹具扩展：fake-acp-agent 新增 `--model-options`/`--reject-model`，`session/set_config_option` journal 加 `set_config_option` 行（带 session_id/config_id/value），按脚本拒绝无效 model。验证：1.2 的两个测试用夹具断言 wire 级行为。

## 2. webui：模型下拉 + create-with-model + 中程切换

- [x] 2.1 `POST /api/sessions` 增加 `model: Option<String>`（`CreateSessionRequest`）；`acp_spawn_and_activate`/`acp_resume_and_activate` 增加 `model` 参数，spawn 成功后、首 prompt 前 flush `SetModel`（若有）；失败报非致命错误（会话仍可对话）。验证：真实 opencode 带 model 创建 → 快照 current_model = 请求模型；带无效 model 创建 → 会话仍建立、transcript 呈现非致命错误行。
- [x] 2.2 会话快照/详情暴露 `current_model` + `available_models`：`SessionInfo`/`Mapping`/`MappingDto`（serde default）三层 + `SessionRow`；来源是 spawn outcome 的 `AcpModelInfo`（随 `activate` 写入映射）。验证：快照 API 字段存在；真实 opencode 会话 `current_model`+34 个 `available_models`；无模型选项 agent（Claude 驱动 / mock 无 configOptions）→ 两字段 null、前端不显示下拉。
- [x] 2.3 中程切换：`POST /api/sessions/{key}/model`（`set_session_model` trait 方法，InProcessBackend→`Out::SendAcp SetModel`，CoreChannelBackend→通道 `SetSessionModel` 请求，DualSessionBackend 转发 acp 侧）；会话详情模型选择器 → 快照 current_model 更新；agent 拒绝时 UI 显示错误（transcript `❌ set model ...`）、模型不变。验证：真实 opencode 切换成功（`⚙ model → ...` + current_model 更新）+ 拒绝路径（错误行 + current 不变）。

## 3. 回归 + 真实验收

- [x] 3.1 `cargo test --workspace` 全绿（唯一失败为 pre-existing `permission_card_snapshot` 快照漂移，sebas-feishu，本 change 未触碰）+ `cargo build --workspace` 无新 warning。验证：`sebas-acp` 53、`sebas-router` 73+21+29+25+… 全绿、`sebas-webui` 全套全绿、根 crate 测试全绿（41 个 test result ok 包），唯一 `permission_card_snapshot` FAILED（既有漂移）。前端 `pnpm run build` 通过、`pnpm run test` 98 通过。
- [x] 3.2 沙箱真实 opencode 冒烟（已于 2026-09-04 完成，证据已补 `docs/acp-opencode-smoke.md` "模型选择已打通"）：建会话 → 模型下拉列出 opencode 模型（34 个，含 free）→ `POST /model` 选 `opencode/mimo-v2.5-free` → 快照 `current_model` 更新 → 后续 prompt 生效；`set_config_option` 真实返回记录（成功：响应回显含最新 currentValue 的 configOptions，driver 据此刷新本地 current；拒绝：`Invalid params: model not found: ...`）。
- [x] 3.3 无 model 选项 agent：Claude 驱动 / mock 无 configOptions → 快照无模型字段（`spawn_without_model_option_reports_none` 覆盖 Claude 与 mock）、前端无下拉（composer test `hides the model dropdown when no session exposes available_models`）。沙箱内 claude CLI 无凭据诚实死亡（既有限制，非模型路径问题）。

## 4. 收尾

- [x] 4.1 `openspec validate --changes` 通过（含 `acp-model-selection` 新能力 + `acp-driver` delta）。验证：见下方备注。
- [x] 4.2 冒烟证据已补 `docs/acp-opencode-smoke.md`（模型下拉 + 切换 + free 模型可用性 + 拒绝语义）；`add-state-store` 的 sessions 表字段（`current_model` 列）标注已在其 tasks 3.1（2026-09-04 已有对应注意项），不代改。

> **验证备注**（2026-09-04）：本 change 全部任务完成。`openspec validate --changes` 已验证通过（`acp-model-selection` 能力 + `acp-driver` delta）。真实 opencode 端到端：create-with-model、中程切换（`session/set_config_option` 用真实 ACP 会话 id）、无效模型拒绝均按预期。一个实现期发现并修复的问题记录：中程 `current_model` 快照更新原只挂在 `apply_event_to_out`（即时路径）；流式 pump 走 `apply_event`，故在 `apply_event` 里补了 `ModelChanged → set_current_model + publish_updated`，两者现在都生效（沙箱验证 current_model 更新 + transcript `⚙`/`❌` 行）。