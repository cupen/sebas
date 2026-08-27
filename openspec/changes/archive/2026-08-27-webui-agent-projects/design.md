# Design: webui-agent-projects

## Context

现有代码中已有：
- `webui/templates/agent.html` — 完整的项目导向 UI 骨架（sidebar + chat + composer）
- `router/src/state.rs:Mapping.project_dir` — 映射级存储字段
- `router/src/router/mod.rs:web_spawn(prompt, project_dir)` — 接受 project_dir 的 spawn 入口
- `webui/src/models.rs:SessionRow` — 尚无 `project_dir` / `prompt_preview` 字段

缺口：`/agent` 和 `/api/agent/*` 路由从未注册，`agent.html` 模板处于 dead 状态。
详见 proposal.md。

## Goals / Non-Goals

**Goals**

- 注册 `/agent` 页面路由 + `/api/agent/*` API 路由，激活 agent.html 模板。
- 用户输入 git 仓库路径 → 创建 session 并设 project_dir → 在 WebUI 中对话。
- SessionRow 携带 project_dir / prompt_preview 供模板渲染。

**Non-Goals**

- 不做文件系统目录浏览/选择器。
- 不做 git 仓库状态检测（是否 dirty、是否有 remote）。
- 不修改现有 session 路由的行为。
- 不涉及 agent 子进程的 IPC 增强。

## Decisions

### D1: 新路由注册在 server.rs，不拆分新模块

`/agent` 和 `/agent/{key}` 是页面路由，`/api/agent/*` 是 API 路由，
全部注册在 `server.rs` 的 `build_router_with_admin_adapter` 中。

备选：在 `routes.rs` 中新增 `routes_for_agent()` 返回子 router 再 merge。
但现有 routes.rs 已是页面/API 混排，保持一致性 > 过度拆分。

### D2: project_dir 路径验证限定为「存在且是目录」

`POST /api/agent/projects` 接收路径后：
1. 展开 `~`（复用 `crate::config::expand_tilde`）
2. 调用 `std::path::Path::exists()` + `is_dir()`
3. 失败 → 400 error

不检查 git 仓库特征（`.git` 子目录）—— 用户可能用非 git 目录工作，
限制太严格无收益。

### D3: SessionRow 的 project_dir 从 Mapping 读取

`routes.rs` 的 session list / detail 函数在构建 `SessionRow` 时，
从 `Mapping::project_dir` 读取并填充。`prompt_preview` 从 session 的
首条用户消息截取（已有 `user_prompt` 字段）。

数据流：`web_spawn(prompt, project_dir)` → `Out::WebSpawn` →
`dispatch_out` 处理 → ACP spawn → session 状态追踪 → state 文件持久化
→ WebUI 从 state 恢复后重建 Mapping（含 project_dir）。

### D4: 直接复用 agent.html 模板，不做大改

现有 `agent.html` 模板已引用 `sessions`、`active_session`、`active_key`
等变量。只需在 Rust handler 中构造匹配的 JSON context 传给它即可。
`base.html` 侧栏导航加入 Agent tab 链接。

### D5: 使用 HTMX 轮询 timeline（与现有模式一致）

现有 session detail 页面使用 `hx-get="/sessions/{key}"` 全页刷新。
agent 页面改为 `hx-get="/agent/{key}/timeline"` 增量更新 timeline，
与模板中已有的 `hx-trigger="every 3s"` 一致。

## Risks / Trade-offs

- [agent 页面与 dashboard/sessions 页面数据重复] → 两者共享同一
  SessionMap 和 router，数据一致。额外渲染开销可忽略。
- [模板中引用的 `prompt_preview` 字段在现有 SessionRow 中不存在]
  → 新增字段，默认回退到 `chat_id`。
- [路径输入不含 `~` 时展开无影响] → `expand_tilde` 直接返回原值。

## Migration Plan

1. `SessionRow` 新增 `project_dir` / `prompt_preview`。
2. `routes.rs` 新增 handler 函数。
3. `server.rs` 注册新路由。
4. 模板侧栏加入 Agent tab。
5. 测试：`cargo test -p webui` 全绿，手动访问 `/agent` 确认渲染。

## Open Questions

（无）