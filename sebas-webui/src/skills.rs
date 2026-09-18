//! Skills 管理面（add-agent-skills 5.1/5.3，design D4）：webui 对操作者级
//! skill 仓是「查看与移除的窗口，不是编辑器」——列表、预览、删除、投影
//! 触发，绝不提供 create/edit（spec 明令；创建与修改走 CLI 或社区工具，
//! 之后用户点刷新）。
//!
//! 分层（与 [`crate::admin::AdminAdapter`] / [`crate::session_backend::
//! SessionBackend`] 同一缝合法）：sebas-webui crate 定义 wire 类型与
//! [`SkillsService`] trait，文件系统实现在主 crate（`sebas::skills::
//! FsSkillsService`，复用同一份 core 逻辑，CLI 与 webui 两调用方共用）。
//! webui crate 不能反向依赖主 crate，故此处只有接缝。
//!
//! 端点（全部挂 `/api/skills`）：
//!
//! | 方法 | 路径 | 语义 |
//! |---|---|---|
//! | GET | `/api/skills` | 扫仓 → `{skills: [entry]}`；目录缺失 = 空仓（空列表，不是错误） |
//! | GET | `/api/skills/{name}` | SKILL.md 原文（`text`，不渲染）+ attachment 文件名列表；未知 → 404 |
//! | DELETE | `/api/skills/{name}` | 只删仓内条目，**不动任何 backend**（清理归下一次 sync）；未知 → 404 |
//! | POST | `/api/skills/sync` | 全量 reconcile → 每 backend 报告 + no_placement（spec「reported, not skipped」） |
//!
//! `name` 参数一律先过 [`is_safe_skill_name`]（拒绝 `/` `\` `..` 等），防止
//! 目录穿越。

use crate::server::WebUiState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// `GET /api/skills` 的一行（spec「list view」形状）。`reason` 只在 invalid
/// 时出现在 wire 上（`reason?`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillEntry {
    pub name: String,
    /// frontmatter `description`（invalid 时为 null）。
    pub description: Option<String>,
    /// 随附文件（相对路径，`/` 分隔，已排序）。
    pub attachments: Vec<String>,
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `GET /api/skills/{name}` 的响应体：SKILL.md **原文**（前端渲染 markdown，
/// 后端不渲染）。条目在仓但缺 SKILL.md（invalid）时 `text` 为 null——诚实
/// 呈现而不是 404 冒充不存在。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillDetail {
    pub name: String,
    pub text: Option<String>,
    pub attachments: Vec<String>,
}

/// 一次 sync 对一个 backend 落点的报告（design D4 wire 形状）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackendSyncReport {
    pub backend: String,
    pub written: Vec<String>,
    /// 同名覆盖（含用户手改——「仓 wins」是 spec 明定语义，必须如实呈现）。
    pub overwritten: Vec<String>,
    pub deleted: Vec<String>,
    /// 名外条目计数（用户私产：不读不导不动，只报数）。
    pub private_ignored: usize,
}

/// `POST /api/skills/sync` 的响应体。`no_placement` 里的 configured backend
/// 没有落点约定——如实报告而不是静默跳过（spec「reported, not skipped」）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillsSyncOutcome {
    pub reports: Vec<BackendSyncReport>,
    pub no_placement: Vec<String>,
}

/// 仓操作接缝（add-agent-skills 5.1）：实现在主 crate（`sebas::skills::
/// FsSkillsService`），测试可注入假件。方法都是同步的——操作对象是本机
/// 文件系统的小目录，与 projects/archive 面的同步 fs 读同款。文件系统失败
/// 一律如实上抛（handler 映射 500），绝不冒充成功或「不存在」。
pub trait SkillsService: Send + Sync {
    /// 扫仓（目录缺失 = 空仓 → 空列表）。
    fn list(&self) -> Vec<SkillEntry>;
    /// 条目详情（SKILL.md 原文 + attachments）；不存在 → None。
    fn detail(&self, name: &str) -> Option<SkillDetail>;
    /// 只删仓内条目（不动 backend）。`Ok(true)` = 已删；`Ok(false)` =
    /// 不存在（handler → 404）；`Err` = 文件系统失败（handler → 500）。
    fn delete(&self, name: &str) -> Result<bool, String>;
    /// 全量投影：对每个有落点的 configured backend 跑 reconcile。`Err` =
    /// 投影中途文件系统失败——报告不完整宁可整体失败（handler → 500）。
    fn sync(&self) -> Result<SkillsSyncOutcome, String>;
}

/// 未接线时的缺省实现（最小入口 / 未升级的装配形态）：仓按空仓处理、
/// 名字一律不存在、sync 无 backend 可投影——诚实退化，绝不假装成功写了
/// 什么。生产装配点（webui_cmd / run）恒注入真实现。
pub struct UnwiredSkills;

impl SkillsService for UnwiredSkills {
    fn list(&self) -> Vec<SkillEntry> {
        Vec::new()
    }

    fn detail(&self, _name: &str) -> Option<SkillDetail> {
        None
    }

    fn delete(&self, _name: &str) -> Result<bool, String> {
        Ok(false)
    }

    fn sync(&self) -> Result<SkillsSyncOutcome, String> {
        Ok(SkillsSyncOutcome {
            reports: Vec::new(),
            no_placement: Vec::new(),
        })
    }
}

/// 仓条目名的安全阀（与 core `skills::is_safe_entry_name` 同标准的 webui 侧
/// 镜像——两 crate 不能共享代码，改动必须两侧同步）：拒绝空名、`.`/`..`、
/// 路径分隔符与 NUL——`{name}` 路径参数绝不拼出仓目录之外的路径。
pub fn is_safe_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0'])
        && std::path::Path::new(name)
            .file_name()
            .is_some_and(|n| n == name)
}

/// 统一 JSON 错误体（api.rs 同款形状 `{error}`；api_error 为 crate 内私有，
/// 这里独立一份避免为两个 handler 翻动整个 api.rs 的可见性）。
fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

/// GET /api/skills — 扫仓列表。
pub async fn skills_list(State(state): State<WebUiState>) -> Response {
    Json(serde_json::json!({ "skills": state.skills.list() })).into_response()
}

/// GET /api/skills/{name} — SKILL.md 原文 + attachments。穿越名 → 400，
/// 不存在 → 404（先安全后存在性，不借 404 探测路径）。
pub async fn skills_detail(State(state): State<WebUiState>, Path(name): Path<String>) -> Response {
    if !is_safe_skill_name(&name) {
        return api_error(StatusCode::BAD_REQUEST, format!("非法的 skill 名 {name:?}"));
    }
    match state.skills.detail(&name) {
        Some(detail) => Json(detail).into_response(),
        None => api_error(StatusCode::NOT_FOUND, format!("仓里没有条目 {name:?}")),
    }
}

/// DELETE /api/skills/{name} — 只删仓（backend 清理归下一次 sync，前端确认
/// 文案负责讲明）。穿越名 → 400，不存在 → 404，文件系统失败 → 500。
pub async fn skills_delete(State(state): State<WebUiState>, Path(name): Path<String>) -> Response {
    if !is_safe_skill_name(&name) {
        return api_error(StatusCode::BAD_REQUEST, format!("非法的 skill 名 {name:?}"));
    }
    match state.skills.delete(&name) {
        Ok(true) => Json(serde_json::json!({ "status": "deleted", "name": name })).into_response(),
        Ok(false) => api_error(StatusCode::NOT_FOUND, format!("仓里没有条目 {name:?}")),
        Err(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("删除 {name:?} 失败：{e}"),
        ),
    }
}

/// POST /api/skills/sync — 全量投影，返回每 backend 报告 + no_placement。
/// 投影中途失败（部分 backend 可能已写）→ 500，如实上抛。
pub async fn skills_sync(State(state): State<WebUiState>) -> Response {
    match state.skills.sync() {
        Ok(outcome) => Json(outcome).into_response(),
        Err(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("skills sync 失败：{e}"),
        ),
    }
}
