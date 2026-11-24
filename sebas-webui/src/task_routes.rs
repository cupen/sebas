//! 项目任务中心 HTTP API（agent-task-center 2.x / 3.x）.
//!
//! 每个 handler 都是薄壳：项目 id → 项目目录 → `bd` 调用 → 类型化结果。
//! 三条边界纪律：
//!
//! - **初始化惰性且幂等**：任务面板首次命中时 `ensure_initialized`（无
//!   `.beads/` 才 `bd init`；无论如何强制 `events-journal = true`），失败
//!   以 503 + `bd` stderr 原文呈现，不 panic、不假装空列表。
//! - **词表在 WebUI 边界校验**（design D10）：assignee / reviewer 必须 ∈
//!   已注册用户名 ∪ 已配置 agent kind，否则 400 且**不调** `bd`。
//! - **评审闸门在 WebUI 层**（design D4/D10）：close 仅人或 issue 的
//!   `reviewer` 元数据点名的 agent 可调；其他 agent 403，beads 不受影响。

use crate::api::{api_error, normalize_project_rows};
use crate::auth::Identity;
use crate::beads_client::{BeadsError, BeadsIssue, CreateTaskRequest, DEFAULT_TIMEOUT, TaskFilter};
use crate::server::WebUiState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// 评审标签（beads 原生 status 之外用 label 表达 `in_review`——design D4）。
pub const IN_REVIEW_LABEL: &str = "in-review";

/// 已解析的项目：wire 标识 + 本机目录。
pub(crate) struct ProjectCtx {
    pub(crate) id: String,
    pub(crate) dir: PathBuf,
}

/// 调用者身份：webui 登录名（空 = 鉴权未启用，`bd` 走自己的 actor 解析）。
struct Caller {
    name: String,
}

impl Caller {
    fn from_identity(identity: Option<axum::extract::Extension<Identity>>) -> Self {
        Self {
            name: identity
                .map(|axum::extract::Extension(identity)| identity.username)
                .unwrap_or_default(),
        }
    }

    /// 非空才作为 `--actor` 下发（避免把空串写进审计）。
    fn bd_actor(&self) -> &str {
        &self.name
    }
}

/// 项目 id → 目录。store 不可达是 503 + cause，未知 id / 越界 / 远程项目
/// 一律 404（与项目列表的隐藏语义一致，不回显服务端路径）。
pub(crate) async fn lookup_project(
    state: &WebUiState,
    raw_id: &str,
) -> Result<ProjectCtx, (StatusCode, String)> {
    let id = urlencoding::decode(raw_id)
        .map(|d| d.into_owned())
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid id encoding".to_string()))?;
    let entries = match state.backend.state_snapshot("projects").await {
        Some(snapshot) => normalize_project_rows(snapshot),
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "core 状态库不可达，项目注册表不可用".to_string(),
            ));
        }
    };
    let Some(entry) = entries
        .into_iter()
        .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
    else {
        return Err((StatusCode::NOT_FOUND, "project not found".to_string()));
    };
    let node = entry
        .get("node_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(crate::projects::LOCAL_NODE_ID);
    if node != crate::projects::LOCAL_NODE_ID {
        // v1 单机范围（spec「Single-machine multi-agent scope」）：远端项目的
        // `.beads/` 不在这台机器上，无从 spawn。
        return Err((
            StatusCode::BAD_REQUEST,
            "远程项目暂不支持任务中心（v1 单机范围）".to_string(),
        ));
    }
    let Some(dir) = entry
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    else {
        return Err((StatusCode::NOT_FOUND, "project not found".to_string()));
    };
    // 越界项目在列表中本就隐藏（add-workspace-root 2.2）：任务面同罪 404。
    if crate::fs::stored_path_in_workspace_root(&dir, &state.workspace_root) != Some(true) {
        return Err((StatusCode::NOT_FOUND, "project not found".to_string()));
    }
    Ok(ProjectCtx {
        id,
        dir: PathBuf::from(dir),
    })
}

#[allow(clippy::result_large_err)] // axum Response 是 handler 错误面的常态形状
async fn resolve_project(state: &WebUiState, raw_id: &str) -> Result<ProjectCtx, Response> {
    lookup_project(state, raw_id)
        .await
        .map_err(|(status, message)| api_error(status, message))
}

/// 惰性初始化（2.1~2.3）：失败 503 + `bd` stderr 原文，面板据此内联呈现。
/// 只在项目首次访问时真正跑 `bd`（P3-2：hub 按项目缓存已确保标记）。
#[allow(clippy::result_large_err)] // axum Response 是 handler 错误面的常态形状
async fn ensure_ready(
    hub: &crate::task_events::TaskEventHub,
    project: &ProjectCtx,
) -> Result<(), Response> {
    hub.ensure_ready(&project.id, &project.dir)
        .await
        .map_err(|error| {
            let detail = beads_detail(&error);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": format!("beads 初始化失败: {detail}"),
                    "code": "beads_init_failed",
                    "detail": detail,
                })),
            )
                .into_response()
        })
}

/// `bd` 失败 → HTTP。1.4 的映射表：
/// NotFound 404 / 未知词表 400（handler 直接拒） / 认领冲突 409 /
/// 命令失败与解析失败 500 / 未安装与超时 503。
fn beads_error_response(error: BeadsError) -> Response {
    match &error {
        BeadsError::NotFound { id } => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!("任务不存在: {id}"),
                "code": "task_not_found",
            })),
        )
            .into_response(),
        BeadsError::AlreadyClaimed { holder } => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("任务已被 {holder} 认领"),
                "code": "already_claimed",
                "holder": holder,
            })),
        )
            .into_response(),
        BeadsError::JournalTruncated { floor, head } => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("beads events journal 已截断（floor={floor}, head={head}）"),
                "code": "events_journal_truncated",
                "floor": floor,
                "head": head,
            })),
        )
            .into_response(),
        BeadsError::NotInstalled(detail) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": format!("bd 未安装或不可执行: {detail}"),
                "code": "bd_not_installed",
                "detail": detail,
            })),
        )
            .into_response(),
        BeadsError::Timeout => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "bd 命令超时",
                "code": "bd_timeout",
            })),
        )
            .into_response(),
        BeadsError::CommandFailed { code, stderr, .. } => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("bd 命令失败 (exit {code}): {stderr}"),
                "code": "bd_command_failed",
                "detail": stderr,
            })),
        )
            .into_response(),
        BeadsError::Parse(detail) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("bd 输出无法解析: {detail}"),
                "code": "bd_parse_error",
                "detail": detail,
            })),
        )
            .into_response(),
    }
}

/// 失败原文（stderr 优先）：初始化错误与命令错误的 detail 同源。
fn beads_detail(error: &BeadsError) -> String {
    match error {
        BeadsError::CommandFailed { stderr, .. } if !stderr.trim().is_empty() => stderr.clone(),
        other => other.to_string(),
    }
}

/// agent 身份集合（P2-1）：config 种子 `[acp.agents.*]` ∪ Settings → Agents
/// 的 store 行。**评审闸门与 assignee / reviewer 词表共用这一处**——只查
/// config 会把 store 里建的 agent kind 判成人类，绕过 close 闸门。
async fn agent_principals(state: &WebUiState) -> BTreeSet<String> {
    let mut set: BTreeSet<String> = state
        .agent_kinds
        .agent_kind_ids()
        .await
        .into_iter()
        .collect();
    if let Some(snapshot) = state.backend.state_snapshot("agents").await
        && snapshot.get("error").is_none()
    {
        set.extend(store_agent_ids(&snapshot));
    }
    set
}

/// store 快照里的活跃 agent id（墓碑行由 `AgentRow::snapshot_value` 的
/// `agents` 数组排除，这里只取 id）。
fn store_agent_ids(snapshot: &serde_json::Value) -> Vec<String> {
    snapshot
        .get("agents")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("id").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

/// 词表（design D10）：已注册用户名 ∪ agent 身份集合（见
/// [`agent_principals`]）。用户库不可达是 503（无法校验就不放行）。
#[allow(clippy::result_large_err)] // axum Response 是 handler 错误面的常态形状
async fn known_principals(state: &WebUiState) -> Result<BTreeSet<String>, Response> {
    let mut set = BTreeSet::new();
    if let Some(store) = state.auth.user_store() {
        let users = store.list().map_err(|error| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": format!("用户库不可达，无法校验 assignee: {error}"),
                    "code": "user_store_unavailable",
                })),
            )
                .into_response()
        })?;
        set.extend(users.into_iter().map(|user| user.username));
    }
    set.extend(agent_principals(state).await);
    Ok(set)
}

/// 词表校验：`None`/空值放行；未知 principal 400 且不调 `bd`。
#[allow(clippy::result_large_err)] // axum Response 是 handler 错误面的常态形状
fn validate_principal(
    field: &str,
    value: Option<&str>,
    known: &BTreeSet<String>,
) -> Result<(), Response> {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(());
    };
    if known.contains(value) {
        return Ok(());
    }
    Err(api_error(
        StatusCode::BAD_REQUEST,
        format!("未知 {field}: {value}（必须是已注册用户名或已配置的 agent kind）"),
    ))
}

fn body_str<'a>(body: &'a serde_json::Value, field: &str) -> Option<&'a str> {
    body.get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn task_filter(query: &TaskQuery) -> TaskFilter {
    TaskFilter {
        status: query.status.clone(),
        assignee: query.assignee.clone(),
    }
}

/// `GET /api/projects/{id}/tasks?status=&assignee=`（3.1）。
#[derive(Debug, Default, serde::Deserialize)]
pub struct TaskQuery {
    pub status: Option<String>,
    pub assignee: Option<String>,
}

pub async fn tasks_list(
    State(state): State<WebUiState>,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<TaskQuery>,
) -> Response {
    let project = match resolve_project(&state, &id).await {
        Ok(project) => project,
        Err(response) => return response,
    };
    if let Err(response) = ensure_ready(&state.tasks, &project).await {
        return response;
    }
    match state
        .tasks
        .client()
        .list(&project.dir, &task_filter(&query))
        .await
    {
        Ok(tasks) => Json(json!({ "tasks": tasks })).into_response(),
        Err(error) => beads_error_response(error),
    }
}

/// `POST /api/projects/{id}/tasks`（3.2）：title 必填；assignee/reviewer
/// 走词表校验（reviewer 落 `--metadata`）。
pub async fn tasks_create(
    State(state): State<WebUiState>,
    Path(id): Path<String>,
    identity: Option<axum::extract::Extension<Identity>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let project = match resolve_project(&state, &id).await {
        Ok(project) => project,
        Err(response) => return response,
    };
    let Some(title) = body_str(&body, "title") else {
        return api_error(StatusCode::BAD_REQUEST, "title 必填");
    };
    let known = match known_principals(&state).await {
        Ok(known) => known,
        Err(response) => return response,
    };
    let assignee = body_str(&body, "assignee");
    let reviewer = body_str(&body, "reviewer");
    for (field, value) in [("assignee", assignee), ("reviewer", reviewer)] {
        if let Err(response) = validate_principal(field, value, &known) {
            return response;
        }
    }
    if let Err(response) = ensure_ready(&state.tasks, &project).await {
        return response;
    }
    let request = CreateTaskRequest {
        title: title.to_string(),
        description: body_str(&body, "description").map(str::to_string),
        assignee: assignee.map(str::to_string),
        reviewer: reviewer.map(str::to_string),
        labels: Vec::new(),
    };
    let caller = Caller::from_identity(identity);
    match state
        .tasks
        .client()
        .create(&project.dir, caller.bd_actor(), &request)
        .await
    {
        Ok(issue) => (StatusCode::CREATED, Json(issue)).into_response(),
        Err(error) => beads_error_response(error),
    }
}

/// `GET /api/projects/{id}/tasks/{task_id}`（3.3）：含评论。
pub async fn tasks_show(
    State(state): State<WebUiState>,
    Path((id, task_id)): Path<(String, String)>,
) -> Response {
    let project = match resolve_project(&state, &id).await {
        Ok(project) => project,
        Err(response) => return response,
    };
    if let Err(response) = ensure_ready(&state.tasks, &project).await {
        return response;
    }
    match state.tasks.client().show(&project.dir, &task_id).await {
        Ok(issue) => Json(issue).into_response(),
        Err(error) => beads_error_response(error),
    }
}

/// `POST .../{task_id}/claim`（3.4）：beads 原子认领；同 actor 幂等 200，
/// 他人活认领 409 点名持有者。
pub async fn tasks_claim(
    State(state): State<WebUiState>,
    Path((id, task_id)): Path<(String, String)>,
    identity: Option<axum::extract::Extension<Identity>>,
) -> Response {
    let project = match resolve_project(&state, &id).await {
        Ok(project) => project,
        Err(response) => return response,
    };
    if let Err(response) = ensure_ready(&state.tasks, &project).await {
        return response;
    }
    let caller = Caller::from_identity(identity);
    match state
        .tasks
        .client()
        .claim(&project.dir, caller.bd_actor(), &task_id)
        .await
    {
        Ok(issue) => Json(issue).into_response(),
        Err(error) => beads_error_response(error),
    }
}

/// `POST .../{task_id}/release`（3.5）：清空 assignee、status 回 `open`。
pub async fn tasks_release(
    State(state): State<WebUiState>,
    Path((id, task_id)): Path<(String, String)>,
    identity: Option<axum::extract::Extension<Identity>>,
) -> Response {
    let project = match resolve_project(&state, &id).await {
        Ok(project) => project,
        Err(response) => return response,
    };
    if let Err(response) = ensure_ready(&state.tasks, &project).await {
        return response;
    }
    let caller = Caller::from_identity(identity);
    match state
        .tasks
        .client()
        .unclaim(&project.dir, caller.bd_actor(), &task_id)
        .await
    {
        Ok(issue) => Json(issue).into_response(),
        Err(error) => beads_error_response(error),
    }
}

/// `POST .../{task_id}/assign`（3.6）：`{"assignee":"..."}`，词表校验。
pub async fn tasks_assign(
    State(state): State<WebUiState>,
    Path((id, task_id)): Path<(String, String)>,
    identity: Option<axum::extract::Extension<Identity>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let project = match resolve_project(&state, &id).await {
        Ok(project) => project,
        Err(response) => return response,
    };
    let Some(assignee) = body_str(&body, "assignee") else {
        return api_error(StatusCode::BAD_REQUEST, "assignee 必填");
    };
    let known = match known_principals(&state).await {
        Ok(known) => known,
        Err(response) => return response,
    };
    if let Err(response) = validate_principal("assignee", Some(assignee), &known) {
        return response;
    }
    if let Err(response) = ensure_ready(&state.tasks, &project).await {
        return response;
    }
    let caller = Caller::from_identity(identity);
    match state
        .tasks
        .client()
        .assign(&project.dir, caller.bd_actor(), &task_id, assignee)
        .await
    {
        Ok(issue) => Json(issue).into_response(),
        Err(error) => beads_error_response(error),
    }
}

/// `POST .../{task_id}/review`（3.7）：`--add-label in-review`。
pub async fn tasks_review(
    State(state): State<WebUiState>,
    Path((id, task_id)): Path<(String, String)>,
    identity: Option<axum::extract::Extension<Identity>>,
) -> Response {
    let project = match resolve_project(&state, &id).await {
        Ok(project) => project,
        Err(response) => return response,
    };
    if let Err(response) = ensure_ready(&state.tasks, &project).await {
        return response;
    }
    let caller = Caller::from_identity(identity);
    match state
        .tasks
        .client()
        .add_label(&project.dir, caller.bd_actor(), &task_id, IN_REVIEW_LABEL)
        .await
    {
        Ok(issue) => Json(issue).into_response(),
        Err(error) => beads_error_response(error),
    }
}

/// `POST .../{task_id}/close`（3.8）：评审闸门——人是放行的，agent 必须
/// 是 issue 的 reviewer；`{"reason":"cancelled"}` 记录取消原因。
pub async fn tasks_close(
    State(state): State<WebUiState>,
    Path((id, task_id)): Path<(String, String)>,
    identity: Option<axum::extract::Extension<Identity>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let project = match resolve_project(&state, &id).await {
        Ok(project) => project,
        Err(response) => return response,
    };
    if let Err(response) = ensure_ready(&state.tasks, &project).await {
        return response;
    }
    let caller = Caller::from_identity(identity);
    let issue: BeadsIssue = match state.tasks.client().show(&project.dir, &task_id).await {
        Ok(issue) => issue,
        Err(error) => return beads_error_response(error),
    };
    // agent 身份 = 名字落在 agent 身份集合里（config ∪ store，P2-1）；人类
    // （注册用户名但非 agent kind）恒放行（spec「Human always may close」）。
    let agents = agent_principals(&state).await;
    if !caller.name.is_empty()
        && agents.contains(&caller.name)
        && issue.reviewer() != Some(caller.name.as_str())
    {
        return api_error(
            StatusCode::FORBIDDEN,
            format!("agent {} 不是该任务的 reviewer，无权关闭", caller.name),
        );
    }
    let reason = body_str(&body, "reason");
    // 闸门已在 WebUI 裁决：`--force` 只用来穿过 `bd close` 的
    // 「actor ≠ assignee」自检（人类代关 agent 认领的任务正是这个场景）。
    match state
        .tasks
        .client()
        .close(&project.dir, caller.bd_actor(), &task_id, reason, true)
        .await
    {
        Ok(issue) => Json(issue).into_response(),
        Err(error) => beads_error_response(error),
    }
}

/// `POST .../{task_id}/comment`（3.9）：`{"text":"..."}`。
pub async fn tasks_comment(
    State(state): State<WebUiState>,
    Path((id, task_id)): Path<(String, String)>,
    identity: Option<axum::extract::Extension<Identity>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let project = match resolve_project(&state, &id).await {
        Ok(project) => project,
        Err(response) => return response,
    };
    let Some(text) = body_str(&body, "text") else {
        return api_error(StatusCode::BAD_REQUEST, "text 必填");
    };
    if let Err(response) = ensure_ready(&state.tasks, &project).await {
        return response;
    }
    let caller = Caller::from_identity(identity);
    match state
        .tasks
        .client()
        .comment(&project.dir, caller.bd_actor(), &task_id, text)
        .await
    {
        Ok(comment) => (StatusCode::CREATED, Json(comment)).into_response(),
        Err(error) => beads_error_response(error),
    }
}

/// `POST /api/projects/{id}/tasks/refresh`（4.3）：全量 `bd list` 重载 +
/// 把事件游标钉到 journal 当前 head（手动同步 `bd dolt pull` 后的重新
/// 基线）。响应带 `last_seq` 供前端核对。
pub async fn tasks_refresh(State(state): State<WebUiState>, Path(id): Path<String>) -> Response {
    let project = match resolve_project(&state, &id).await {
        Ok(project) => project,
        Err(response) => return response,
    };
    if let Err(response) = ensure_ready(&state.tasks, &project).await {
        return response;
    }
    let client = state.tasks.client();
    let tasks = match client.list(&project.dir, &TaskFilter::default()).await {
        Ok(tasks) => tasks,
        Err(error) => return beads_error_response(error),
    };
    // journal head = 全量重放的最后一个 seq（`bd` 没有 head 子命令；手动
    // 刷新的代价可接受）。**截断时 `--since 0` 必失败**（P2-2）：错误里
    // 自带的 `head` 就是恢复基线——「从现在开始」照常返回任务列表，而不是
    // 让恢复入口恒 409。
    let head = match client.events_tail(&project.dir, 0, DEFAULT_TIMEOUT).await {
        Ok(records) => records.last().map(|record| record.seq).unwrap_or(0),
        Err(BeadsError::JournalTruncated { head, .. }) => head,
        Err(error) => return beads_error_response(error),
    };
    state.tasks.reset_baseline(&project.id, head).await;
    Json(json!({ "tasks": tasks, "last_seq": head })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    fn known() -> BTreeSet<String> {
        ["admin", "claude"].iter().map(|s| s.to_string()).collect()
    }

    /// 3.2/3.6 验收：词表外的 assignee/reviewer 400（且错误点名 principal）。
    #[test]
    fn unknown_assignee_is_rejected_with_400() {
        let response = validate_principal("assignee", Some("nobody"), &known()).unwrap_err();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = validate_principal("reviewer", Some("ghost"), &known()).unwrap_err();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // 已注册用户名与 agent kind 都放行；None/空放行（未指定）。
        assert!(validate_principal("assignee", Some("admin"), &known()).is_ok());
        assert!(validate_principal("assignee", Some("claude"), &known()).is_ok());
        assert!(validate_principal("assignee", None, &known()).is_ok());
        assert!(validate_principal("assignee", Some("  "), &known()).is_ok());
    }

    /// 1.4 验收：每个 `BeadsError` 变体映射到语义化的 HTTP 状态。
    #[test]
    fn beads_errors_map_to_http_statuses() {
        let cases: Vec<(BeadsError, StatusCode)> = vec![
            (
                BeadsError::NotFound { id: "x".into() },
                StatusCode::NOT_FOUND,
            ),
            (
                BeadsError::AlreadyClaimed {
                    holder: "claude".into(),
                },
                StatusCode::CONFLICT,
            ),
            (
                BeadsError::JournalTruncated { floor: 3, head: 9 },
                StatusCode::CONFLICT,
            ),
            (
                BeadsError::NotInstalled("missing".into()),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (BeadsError::Timeout, StatusCode::SERVICE_UNAVAILABLE),
            (
                BeadsError::CommandFailed {
                    code: 1,
                    stdout: String::new(),
                    stderr: "boom".into(),
                },
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                BeadsError::Parse("bad json".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (error, want) in cases {
            let response = beads_error_response(error);
            assert_eq!(response.status(), want);
        }
    }

    /// P2-1 验收：agent 身份集合 = config 种子 ∪ store 行。只查 config 会把
    /// Settings 里建的 agent kind 判成人类，从而绕过 close 闸门。
    #[test]
    fn agent_principals_union_config_and_store_rows() {
        let config = vec!["claude".to_string()];
        let snapshot = serde_json::json!({
            "agents": [{"id": "codex"}, {"id": "claude"}],
            "deleted_ids": ["gone"],
        });
        let principals = {
            let mut set: BTreeSet<String> = config.into_iter().collect();
            set.extend(store_agent_ids(&snapshot));
            set
        };
        assert!(principals.contains("claude"));
        assert!(
            principals.contains("codex"),
            "store 行必须并进 agent 身份集合（P2-1）: {principals:?}"
        );
        // 不可达的 store 只丢 store 面，config 面照常——不清空、不误判。
        assert_eq!(
            store_agent_ids(&serde_json::json!({"error": "unavailable"})),
            Vec::<String>::new()
        );
    }

    /// 认领冲突的响应体点名持有者（前端据此显示「已被 X 认领」）。
    #[tokio::test]
    async fn claim_conflict_body_names_the_holder() {
        let response = beads_error_response(BeadsError::AlreadyClaimed {
            holder: "alice".into(),
        });
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], "already_claimed");
        assert_eq!(body["holder"], "alice");
    }
}
