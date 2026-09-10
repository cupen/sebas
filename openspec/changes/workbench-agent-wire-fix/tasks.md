## 1. 根因修复：占位会话显式标记

- [x] 1.1 `sebas-dispatch/src/state.rs`：`MappingState::Spawning` 增加 `awaiting_first_prompt: bool` 字段（`#[serde(default)]` 兼容旧记录）；`Mapping::spawning_with`（占位路径）置 true，普通 spawn 置 false；`route_text` 的 Spawning 分支改为只看该标记（不再用 `pending_kind.is_some() || pending_model.is_some()`）。验证：`cargo test -p sebas-dispatch` 新增「占位标记为 true 时首条消息走 SpawnNew（无论 kind/model 是否为空）」用例通过
- [x] 1.2 `sebas-dispatch/src/state.rs`：`spawn` 完成后（`activate`）清掉 `awaiting_first_prompt`；落盘/重载的 round-trip 测试覆盖新字段。验证：`cargo test -p sebas-dispatch` round-trip 用例通过
- [x] 1.3 回归锁：补一条「`backend=null` 占位 + 首条消息 → SpawnNew」与「`backend="acp"`（旧值，现为无效值）占位 + 首条消息 → SpawnNew」的 core_channel 测试（在 D2 切换后改为 `agent=<任意有效值>`）。验证：`cargo test --test state_subscription_test` 与 `src/core_channel/tests.rs` 相关用例通过

## 2. Wire 重命名：agent / project_id（BREAKING）

- [x] 2.1 `sebas-webui/src/api.rs`：`CreateSessionRequest` 的 `backend: Option<String>` 改为 `agent: String`（必填）；`project_dir` 改 `project_id`；旧字段与旧值（`"acp"`/`"acp:*"`/null）一律 400。验证：`cargo test -p sebas-webui` 新增「缺 agent → 400」「`agent="claudecode"` → 201」用例通过
- [x] 2.2 `src/agent_backend.rs`：`validate_backend_hint` 改为按 `[acp.agents.*]` 键名 ∪ `{"native"}` 校验 `agent`；`route`/spawn 路径用 agent id 查 `command_for`。验证：`cargo test -p sebas` 相关用例通过
- [x] 2.3 `src/core_channel/server.rs` 与 `client.rs`：`Spawn`/`CreatePlaceholder` 帧的字段名随 wire 同步（`agent`、`project_id`）。验证：`cargo test -p sebas` core_channel 测试通过
- [x] 2.4 `src/projects.rs` + `src/sebas_state/migration.rs`：`projects` 表加 `id TEXT`（`proj-<12hex>`，path SHA-256 前 12 hex）与 `default_agent TEXT` 列；`projects.json` 同步加 `id`/`default_agent` 字段，旧文件启动回填。验证：`cargo test -p sebas` migration 用例通过；旧格式 json 启动后自动补 id
- [x] 2.5 项目相关端点路径参数从 encoded path 改为 `project_id`（`/api/projects/{id}/remove`、`/{id}/branch`）；会话行/详情的 `project_dir` 字段改 `project_id`。验证：`cargo test -p sebas-webui` 端点测试通过
- [x] 2.6 `POST /api/sessions` 成功创建带 `project_id` 的会话后，更新该项目的 `default_agent`。验证：新增「创建会话后 `GET /api/projects` 该项目 `default_agent` 更新」用例通过

## 3. `/api/agents` 唯一真源

- [x] 3.1 `sebas-webui/src/agent_kinds.rs`：`AgentKindInfo` 形状改为 `{id, display, reachable, cause?, version?}`；`discover_agent` 保持探测逻辑，display 按 D3 规则推导。验证：`cargo test -p sebas-webui` 单测通过
- [x] 3.2 `sebas-webui/src/api.rs`：`GET /api/agent-kinds` 改名 `GET /api/agents`；响应含 native 一行（`id="native"`，reachable/cause 来自 native 凭据探测）；`driver` 不在响应中。验证：端点测试断言响应无 `driver` 字段且含 native 行
- [x] 3.3 `/api/agent-defaults` 端点删除（settings 的 provider/model 默认展示改走 `/router/api/providers` + defaults，前端另议）。验证：`cargo test -p sebas-webui` 断言该路由 404
- [x] 3.4 CLI `src/agent_kinds.rs`：`sebas agent-kinds list` 输出列跟随新形状（id/display/reachable/cause/version）。验证：`cargo run -- agent-kinds list` 在沙箱输出符合新形状

## 4. 前端：wire 切换 + composer 跟随/创建

- [x] 4.1 `api/client.ts`：`createSession` 签名改 `{prompt?, project_id?, agent, model?}`；`Project` 类型加 `id`/`default_agent`；`AgentKindInfo` 改新形状；`agentKinds()` 改调 `/api/agents`；删除 `agentDefaults`/`setAgentDefaults`。验证：`pnpm --dir sebas-webui/frontend exec tsc --noEmit` 通过
- [x] 4.2 `views/workbench-composer.ts`：创建模式 agent 下拉数据源改 `/api/agents`（含 native 行，不可达禁选+cause）；`agent` 必填（未选禁提交）；进入项目创建模式时按 `default_agent` 预选；跟随模式 agent 标签加锁定图标+title。验证：`workbench-composer.test.ts` 新增/更新用例通过
- [x] 4.3 `views/project-rail.ts`：`+` 创建占位会话传 `agent`（当前项目 `default_agent` 或首个可达 agent）；创建后**留在工作台路由**（不再 navigate 深链），依赖 `set_focus` 让 composer 进入跟随模式。验证：`project-rail.test.ts` 新增「占位创建后不 navigate、follow 模式生效」用例通过
- [x] 4.4 `views/session-detail.ts`：agent 标签加锁定图标+title；深链 GET 既有的 `set_focus` 行为不动。验证：`session-detail` 相关用例更新通过
- [x] 4.5 `views/settings-modal.ts`：移除对 `/api/agent-defaults` 的引用（默认 provider/model 展示改走 router providers）。验证：`settings-modal.test.ts` 通过

## 5. 前端：rail 删除入口

- [x] 5.1 `views/project-rail.ts`：项目行 hover 出现 remove 按钮 + wa-dialog 确认（文案含项目名与「存活会话迁入 Inbox」）；确认调 `api.projects.remove(project_id)`；拒绝内联呈现。验证：`project-rail.test.ts` 三路径用例通过
- [x] 5.2 `views/project-rail.ts`：会话行加 close 按钮（inactive 直删、active 需确认、关闭聚焦会话回空态）。验证：`project-rail.test.ts` 用例通过

## 6. e2e 旅程（testsuite-webui 沙箱）

- [x] 6.1 `tests/session-roundtrip.spec.ts`：占位会话首条消息旅程（rail「+」→ 留在工作台 → composer 输入 → 收到 fake agent 响应；三种 agent 值各跑一遍）。验证：`pnpm playwright test session-roundtrip` 通过
- [x] 6.2 `tests/projects.spec.ts`：rail 项目删除 + project_id 引用旅程。验证：`pnpm playwright test projects` 通过
- [x] 6.3 `tests/session-mgmt.spec.ts`：rail 会话删除分级确认旅程。验证：`pnpm playwright test session-mgmt` 通过
- [x] 6.4 `tests/models.spec.ts`：会话 agent 不可变的 UI 锁定提示呈现。验证：`pnpm playwright test models` 通过

## 7. 收尾验证

- [x] 7.1 全量门禁：`cargo test`、`cargo clippy`、`pnpm --dir sebas-webui/frontend test`、`invoke testsuite-webui` 全绿
- [x] 7.2 沙箱人工巡检（`invoke testsuite-webui-sandbox`）：占位发消息、rail 删除、项目默认 agent 预选、agent 锁定提示四条旅程手工过一遍
