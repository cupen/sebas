# Tasks: webui-agent-projects

## 1. 数据模型

- [x] 1.1 `webui/src/models.rs`：`SessionRow` 新增 `project_dir: Option<String>`
  和 `prompt_preview: Option<String>` 字段。验证：`cargo check -p webui` 通过

## 2. 后端路由

- [x] 2.1 `webui/src/routes.rs`：在 session list / detail 构建 `SessionRow` 时
  填入 `project_dir`（从 `Mapping` 读取）和 `prompt_preview`（从 `user_prompt`
  截取前 80 字符）。验证：`cargo test -p webui` 通过

- [x] 2.2 `webui/src/routes.rs`：新增 `agent_page()` handler（`GET /agent`），
  渲染 `agent.html` 模板，传入 `sessions`（全量 SessionRow）、`active_session`
  （当前 focused session）、`active_key`。验证：`cargo check -p webui` 通过

- [x] 2.3 `webui/src/routes.rs`：新增 `agent_detail()` handler（`GET /agent/{key}`），
  渲染 `agent.html` 模板，`active_session` 设为该 key 的 session。验证：
  `cargo check -p webui` 通过

- [x] 2.4 `webui/src/routes.rs`：新增 `agent_timeline()` handler
  （`GET /agent/{key}/timeline`），返回 session 的 card body 元素片段。
  验证：`cargo check -p webui` 通过

- [x] 2.5 `webui/src/routes.rs`：新增 `api_create_project()` handler
  （`POST /api/agent/projects`），接收 `path` 表单字段，展开 `~`，验证
  路径存在且为目录，调用 `state.router.web_spawn(autoprompt, Some(project_dir))`，
  返回 `{ "key": encoded }`（201）。路径不存在时返回 400。
  验证：`cargo test -p webui` 新增单测通过

- [x] 2.6 `webui/src/routes.rs`：新增 `api_agent_message()` handler
  （`POST /api/agent/{key}/message`），接收 `message` 表单字段，调用
  `state.router.web_send_message(key, message)`，返回 timeline 片段。
  验证：`cargo check -p webui` 通过

## 3. 路由注册

- [x] 3.1 `webui/src/server.rs`：在 `build_router_with_admin_adapter` 中注册
  `/agent` (GET)、`/agent/{key}` (GET)、`/agent/{key}/timeline` (GET)、
  `/api/agent/projects` (POST)、`/api/agent/{key}/message` (POST)。
  确保 `/api/agent/*` 在 gateway mutation 路由前的顺序不影响。
  验证：`cargo test -p webui` 通过

## 4. 模板

- [x] 4.1 `base.html` 导航栏已含 `<a href="/agent">Agent</a>` 链接（先前遗留）。
  验证：手动访问 webui 侧栏可见

- [x] 4.2 确认 `agent.html` 模板的 `sessions` 变量与 `SessionRow` 新字段
  匹配（`project_dir`、`prompt_preview`）。验证：`cargo test -p webui` 模板
  渲染测试通过

## 5. 收尾

- [x] 5.1 `cargo test --workspace` + `cargo clippy -p webui` 全绿；
  `openspec validate webui-agent-projects` 通过