## REMOVED Requirements

### Requirement: Agent project page
**Reason**: 嵌套 spec 所描述的 `GET /agent` 项目工作台页面属 HTMX 时代旧面，已被 `webui/spec.md` 的 SPA 工作台（`GET /`、session rail + 项目和 History/Inbox 分组渲染）取代，本嵌套 spec 随之归档。
**Migration**: 见 webui「HTTP route surface」（`GET /` SPA shell、项目/会话分组渲染场景）。

### Requirement: Create project session
**Reason**: `POST /api/agent/projects` 旧端点已被 SPA 工作台的项目注册（`POST /api/projects`）取代，自动 prompt 模板等旧语义不再保留。
**Migration**: 见 webui「HTTP route surface」（`GET /api/projects` / `POST /api/projects` 项目注册集群）。

### Requirement: Agent session message
**Reason**: `POST /api/agent/{key}/message` 旧端点并入现有会话 API 面（`POST /api/sessions/{key}/message`），无独有行为。
**Migration**: 见 webui「Standalone core-client semantics」（会话创建/消息/关闭经会话通道直达 core）。

### Requirement: Agent session detail and timeline
**Reason**: `GET /agent/{key}` 详情页与 `/agent/{key}/timeline` HTMX 轮询片段属旧模板渲染面，已被 SPA 的会话详情与 `/ws` 推送取代，无逐条等价 requirement。
**Migration**: 该面已由 SPA 工作台取代，无逐条等价 requirement。

### Requirement: Session model with project metadata
**Reason**: `project_dir` / `prompt_preview` / `agent_kind` 等会话元数据语义已并入现有会话详情/follow-up composer 与「HTTP route surface」的 SPA 面，会话模型字段不再单独保留。
**Migration**: 语义已被 webui SPA 工作台取代，无独有行为需保留。

### Requirement: Navigation tab
**Reason**: 旧「Agent」导航项指向 `/agent`，该路由已随 SPA 工作台（`/`）消失；主导航语义由现有 SPA 面接管。
**Migration**: 语义已被 webui SPA 工作台取代，无独有行为需保留。

### Requirement: Conversation-area composer is session-scoped
**Reason**: follow-up vs creation 模式下 agent 只读/可创建的语义已并入 `agent-driver` 既有引用及现有 SPA 会话详情/composer 相关 requirement。
**Migration**: 语义已被 webui「HTTP route surface」与 agent-driver 既有引用覆盖，无独有行为需保留。
