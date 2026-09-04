## 1. 显式启用开关（feishu-option）

- [x] 1.1 在 `FeishuConfig` 增加 `#[serde(default)] enabled: Option<bool>`，新增 `fn is_enabled(&self)` = `enabled.unwrap_or_else(|| 双非空)`；保留 `enabled()` 为别名；验证 config 单测覆盖「显式 false + 凭据齐全」「显式 true + 凭据缺失」「缺省 + 凭据空」「缺省 + 凭据齐全」四态
- [x] 1.2 在 `Config::validate`/`validate_runtime` 增加：显式 `enabled=true` 但 app_id 或 app_secret 为空 → 配置错误拒绝启动；验证单测该态返回 Err
- [x] 1.3 在 `run.rs` 将 `feishu_enabled = cfg.feishu.enabled()` 改为 `is_enabled()`（显式开关优先），保留「未启用则跳过飞书、pend WS、走 no-feishu 出站泵」分支；验证集成测试——配置 `enabled=false` + 凭据齐全 → 进程不起 WS、不出站；`enabled=true` + 凭据齐全 → 按旧逻辑接入
- [x] 1.4 更新 `config/config.toml` 示例与 `src/watchdog/services.rs` 生成配置模板加入 `[feishu] enabled` 注释；验证部署文档/示例注明「默认关闭，webui 主控」

## 2. router 原生执行体桥（agent-workbench / feishu-bridge）

- [x] 2.1 定义 `NativeSessionBridge` trait（进程内指向 `sebas_agent::session::SessionManager` 句柄）：`is_native(key)`、`prompt(key, text)`、`answer_permission(...)`；`RouterHandle` 增加可选桥字段（None 时零行为变化）；验证 router 构造单测——无桥时 `dispatch` 行为与现有一致
- [x] 2.2 在 `run.rs` 装配：core 进程持有 `SessionManager`（与 webui 内嵌共享同一份）并注入 router 桥，同时把原生 LLM 通道 env（`SEBAS_AGENT_PROVIDER_*`/`SEBAS_AGENT_GATEWAY_URL`）传入（设计 OQ3：沿用 env，不加配置面）；验证 `run --webui` 会话仍可走 native 后端
- [x] 2.3 在 router text 事件处理中：会话已是 `agent-*` 前缀 → 走桥 `prompt`；新会话 + 默认/配置路由 native → 创建原生会话（`agent-*` key）并 `prompt`，不 emit `Out::SpawnAcp`、不渲染飞书卡片（设计 D2/D3）；验证 router 单测——feishu 文本经桥进入原生会话且无卡片 Out；acp 会话分支行为不变

## 3. 原生会话的 webui 呈现（agent-workbench）

- [x] 3.1 复用 `NativeAgentBackend`（`agent-*` 会话）：原生会话的 `AgentEvent` → `SessionEvent`（transcript/工具轨迹）+ `PermissionNotice`（审查卡）→ webui 事件流；验证 webui 单测/集成——webui 打开的 agent-* 会话显示轨迹与审查卡
- [x] 3.2 权限决策回填：`answer_permission` → `ApproverHub`（allow-once / allow-session / deny + reason，fail-closed 无答即拒）；验证既有 `native_spawn_prompts_and_permission_round_trips` 及 feishu 原生会话的同类测试通过

## 4. 部署形态验证与文档（feishu-option）

- [x] 4.1 验证 watchdog 默认（webui on / core off / gateway off）+ 服务页启 core；确认既有默认单测（`webui_enabled_by_default_and_core_disabled`）继续通过
- [x] 4.2 集成验证（sandbox，按 AGENTS.md 隔离规则）：webui 主控下建会话、feishu 关闭时不接 WS；开启 feishu 后 core 起 WS；feishu 会话出现在 webui `GET /api/sessions`；原生会话在飞书侧只留回执（OQ1 缺省不发，验证「无卡片」）
- [x] 4.3 文档：README/AGENTS 部署段写明「webui 默认主控、feishu 可选、双通道共享会话」；spec 同步主 specs（`feishu-option` 新能力、`agent-workbench`/`feishu-bridge` 修改）——归档时执行

## 5. 验证（quality gates）

- [x] 5.1 `cargo test -p sebas-router -p sebas-agent -p sebas-webui` 全绿（含新单测）
- [x] 5.2 `cargo clippy -p sebas-router -p sebas-agent -p sebas-webui` 无新增警告
- [x] 5.3 完整联调（sandbox）：webui 建会话 + 飞书消息会话共享快照；原生会话轨迹/审查卡走 webui；acp 会话飞书卡片行为不变