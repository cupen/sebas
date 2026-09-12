//! Route handlers: pure helpers for the JSON API + the router BFF proxies.

use crate::models::{SessionRow, SessionStatus};
use crate::server::WebUiState;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use sebas_channels::key::ChannelKey;
use sebas_dispatch::SessionInfo;

// ---- Helper functions ----

/// Build `SessionRow`s from backend session info, returning counts. Marks
/// each row with `is_active` so the client can render the active indicator.
pub(crate) fn build_session_rows(
    infos: &[SessionInfo],
    focused: Option<&ChannelKey>,
) -> (Vec<SessionRow>, usize, usize, usize) {
    let mut active = 0usize;
    let mut dormant = 0usize;
    let mut spawning = 0usize;

    let mut rows: Vec<SessionRow> = infos
        .iter()
        .map(|info| {
            let status: &'static str = match info.status.as_str() {
                "active" => {
                    active += 1;
                    "active"
                }
                "dormant" => {
                    dormant += 1;
                    "dormant"
                }
                _ => {
                    spawning += 1;
                    "spawning"
                }
            };
            let is_active = focused
                .map(|a| a.channel.as_str() == info.channel && a.reference == info.key)
                .unwrap_or(false);
            let derived = SessionStatus::derive(status, info.phase.as_deref().unwrap_or(""))
                // 8.4：有悬空审批的会话**在等**，不是在跑——底层 status 仍是
                // active，呈现必须是 Waiting。
                .with_parked_approvals(
                    info.remote
                        .as_ref()
                        .map(|r| r.parked_approvals)
                        .unwrap_or(0),
                );
            SessionRow {
                // 8.1 会话归属项目按 `(节点, 路径)`：远端会话的 project_dir 若按
                // 本机公式算 id，会挂到「本机同路径项目」下（或一个不存在的 id）。
                project_id: crate::projects::project_id_for_session(info),
                prompt_preview: info.user_prompt.clone(),
                current_model: info.current_model.clone(),
                desired_mode: info.desired_mode.clone(),
                effective_mode: info.effective_mode.clone(),
                available_models: info.available_models.clone(),
                agent_kind: info.agent_kind.clone(),
                backend: info.backend.clone(),
                pending_count: info.pending.len(),
                encoded_key: encode_channel_key(&info.channel, &info.key),
                channel: info.channel.clone(),
                reference: info.key.clone(),
                session_id_short: info
                    .session_id
                    .as_deref()
                    .map(|s| crate::models::middle_truncate(s, 18)),
                session_id: info.session_id.clone(),
                status,
                status_label: derived.label(),
                status_slug: derived.slug(),
                status_glyph: derived.glyph(),
                last_active: format_relative_time(info.last_active_unix),
                last_active_unix: info.last_active_unix,
                is_active,
                remote: info.remote.clone(),
                // rail-declutter-unread 1.2：未读徽标的服务端计数随行下发。
                msg_count: info.msg_count,
            }
        })
        .collect();

    // Sort: focused first, then by most-recent activity. Activity compares
    // the underlying unix timestamps — the rendered relative-time string is
    // for humans, and text-comparing "20694d ago" > "0s ago" inverted the
    // intended order.
    rows.sort_by(|a, b| {
        b.is_active
            .cmp(&a.is_active)
            .then_with(|| b.last_active_unix.cmp(&a.last_active_unix))
    });
    (rows, active, dormant, spawning)
}

/// Compact summary used by the dashboard's focused-session banner. Carries
/// the session's conversation as one ordered entry sequence (same shape as
/// the detail endpoint — workbench-conversation-view 1.4).
pub(crate) fn session_summary(
    info: &SessionInfo,
    entries: &[sebas_dispatch::TurnEntry],
) -> serde_json::Value {
    let derived = SessionStatus::derive(&info.status, info.phase.as_deref().unwrap_or(""))
        .with_parked_approvals(
            info.remote
                .as_ref()
                .map(|r| r.parked_approvals)
                .unwrap_or(0),
        );
    let conversation: Vec<crate::models::ConversationEntryView> = entries
        .iter()
        .map(|e| crate::models::ConversationEntryView {
            position: e.position,
            kind: e.kind.clone(),
            element_type: e.element_type.clone(),
            content: e.content.clone(),
            created_at_unix: e.created_at_unix,
            // workbench-agent-identity-and-process-folds 1.1：工具条目标题
            // 原样透传（None = 旧条目，前端回退通用标签）。
            title: e.title.clone(),
        })
        .collect();
    serde_json::json!({
        "channel": info.channel,
        "reference": info.key,
        "session_id": info.session_id,
        "status": info.status,
        "status_label": derived.label(),
        "status_slug": derived.slug(),
        "status_glyph": derived.glyph(),
        "encoded_key": encode_channel_key(&info.channel, &info.key),
        "current_model": info.current_model,
        "available_models": info.available_models,
        "agent_kind": info.agent_kind,
        // 绑定项目的稳定 id（workbench-agent-wire-fix 2.5）；null = inbox。
        // 8.1：按 `(节点, 路径)` 派生——远端会话不属于本机同路径项目。
        "project_id": crate::projects::project_id_for_session(info),
        // 8.2/8.3/8.4/8.5：节点/状态/成因/mode/悬空审批整块透传（null = 本机）。
        "remote": info.remote,
        // workbench-turn-queue 6.1：聚焦会话的待生效提交全量视图（投递序）。
        "pending": serde_json::to_value(&info.pending).unwrap_or_default(),
        // rail-declutter-unread：聚焦会话的段计数（transcript 标记已读时
        // 推进浏览器读锚用，保证 seam 与徽标一致）。
        "msg_count": info.msg_count,
        // workbench-conversation-view 1.4：与 detail 同形状的有序条目序列。
        "entries": conversation,
    })
}

/// Encode a (channel, reference) pair for use in URLs.
pub(crate) fn encode_channel_key(channel: &str, reference: &str) -> String {
    let raw = format!("{channel}\0{reference}");
    urlencoding::encode(&raw).into_owned()
}

/// Encode a ChannelKey for use in URLs.
pub(crate) fn encode_session_key(key: &ChannelKey) -> String {
    encode_channel_key(key.channel.as_str(), &key.reference)
}

/// Decode a URL-encoded ChannelKey.
pub(crate) fn decode_session_key(encoded: &str) -> Option<ChannelKey> {
    let decoded = urlencoding::decode(encoded).ok()?;
    let (channel, reference) = decoded.split_once('\0')?;
    Some(ChannelKey::new(channel, reference))
}

/// Format a unix timestamp as a relative time string.
pub(crate) fn format_relative_time(unix_ts: i64) -> String {
    let diff = chrono::Utc::now().timestamp() - unix_ts;
    if diff < 60 {
        format!("{diff}s ago")
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

/// Format a Duration as a human-readable string.
pub(crate) fn format_uptime(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {mins}m")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

// ---- router BFF 路由（make-core-own-provider-data 3.2）：provider 管理
// ---- 面改由 core 状态库承载，不再代理 router 进程。路径名与鉴权姿态不变。
//
// 数据面：读 = `backend.state_snapshot(domain)`，写 =
// `backend.state_mutate(domain, payload)`（detached 形态经 core session
// channel，内嵌形态直连引擎——同一 seam）。core 不可达时读 503、写 503，
// 绝不回退陈快照（design D2 consequence）。
//
// mutation 状态码契约（把 core 的 rejection cause 映射回 HTTP 语义）：
// - cause 含域 op 前缀（`put:` / `delete:` / `save:` / `set_defaults` /
//   `clear_defaults` / `providers: ` / `aliases: `）→ 业务拒绝：
//   含「不存在」→ 404，含「已存在」→ 409，其余 → 400；
// - 其余（socket not found / 连接失败 / state store 不可用 …）→ 503。

fn router_client_of(state: &WebUiState) -> crate::router_client::RouterClient {
    let listen = state.router.listen.clone().unwrap_or_default();
    crate::router_client::RouterClient::new(&listen)
}

/// 拉取 providers 域快照；不可达（None）或快照自带 error（引擎降级）→ None。
/// 调用方对 None 如实 503，绝不拿陈数据充数。
async fn providers_snapshot(state: &WebUiState) -> Option<serde_json::Value> {
    let v = state.backend.state_snapshot("providers").await?;
    if v.get("error").is_some() {
        return None;
    }
    Some(v)
}

/// 单个 provider 条目 → admin 列表行（对外形状与旧 router `/admin/providers`
/// 一致：name + preset + 三槽位 + api_key_env + api_key_configured + models）。
/// preset 派生条目的槽位/models 从代码表物化（preset 数据跟随代码）；密钥
/// 永不出现在响应里，只给 `api_key_configured` 布尔。
fn admin_provider_row(
    name: &str,
    item: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    let preset_name = item
        .get("preset")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let preset = preset_name.as_deref().and_then(|pn| {
        sebas_router::config::presets()
            .iter()
            .find(|p| p.name == pn)
    });
    let from_item_or_preset = |key: &str, preset_v: Option<&str>| -> serde_json::Value {
        match item
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| preset_v.map(str::to_string))
        {
            Some(v) => serde_json::Value::String(v),
            None => serde_json::Value::Null,
        }
    };
    // models：条目自带优先，否则 preset 代码表。形状是条目列表（id +
    // 能力标记；redesign-provider-models-settings）：条目对象原样读，遗留
    // 裸字符串 / 逗号分隔字符串归一化为仅隐含 text 的条目。
    let models = item
        .get("models")
        .and_then(models_value_to_entries)
        .filter(|list| !list.is_empty())
        .or_else(|| {
            preset.map(|p| {
                p.models
                    .iter()
                    .map(sebas_router::config::PresetModel::to_entry)
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default();
    // api_key_configured：条目明文 key，或 env 名（条目 api_key_env → preset
    // 默认名）在**本进程**有值。与 spawn 侧密钥解析同一优先级。
    let env_name = item
        .get("api_key_env")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| preset.map(|p| p.api_key_env.to_string()));
    let has_plain = item
        .get("api_key")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|k| !k.is_empty());
    let env_set = env_name
        .as_deref()
        .map(std::env::var)
        .is_some_and(|v| v.is_ok_and(|val| !val.is_empty()));
    serde_json::json!({
        "name": name,
        "preset": preset_name,
        "base_url_anthropic": from_item_or_preset(
            "base_url_anthropic",
            preset.and_then(|p| p.base_url_anthropic),
        ),
        "base_url_openai_chat": from_item_or_preset(
            "base_url_openai_chat",
            preset.and_then(|p| p.base_url_openai_chat),
        ),
        "base_url_openai_responses": from_item_or_preset(
            "base_url_openai_responses",
            preset.and_then(|p| p.base_url_openai_responses),
        ),
        "api_key_env": env_name,
        "api_key_configured": has_plain || env_set,
        "models": models,
        // add-fetch-models：条目上可编辑字段的回填视图——挑选抓取结果的
        // 「普通编辑」PUT 需要完整字段集，否则整体替换会静默丢字段。
        "default_model": item
            .get("default_model")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| serde_json::Value::String(s.to_string()))
            .unwrap_or(serde_json::Value::Null),
        "protocol": item
            .get("protocol")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| serde_json::Value::String(s.to_string()))
            .unwrap_or(serde_json::Value::Null),
        // redesign-provider-models-settings 3.2：模型改名映射在编辑器
        // Advanced 折叠区编辑——投影回传供编辑回填（put 整体替换，回填
        // 防静默丢字段）。
        "model_map": item
            .get("model_map")
            .and_then(serde_json::Value::as_object)
            .filter(|m| !m.is_empty())
            .map(|m| serde_json::Value::Object(m.clone()))
            .unwrap_or(serde_json::Value::Null),
    })
}

/// GET /router/api/providers：provider 列表（core 状态库快照投影，管理页
/// 数据源）。墓碑不出现；config 种子 provider 不在此列（store 是唯一事实
/// 来源，种子-only 机器如实显示空表）。core 不可达 → 503。
pub async fn router_api_providers_list(
    State(state): State<WebUiState>,
) -> axum::response::Response {
    let Some(snapshot) = providers_snapshot(&state).await else {
        return err_503_core_unreachable();
    };
    let providers = snapshot
        .get("providers")
        .and_then(serde_json::Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(name, item)| (name.clone(), item.as_object().cloned().unwrap_or_default()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let deleted: Vec<String> = snapshot
        .get("deleted")
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let out: Vec<serde_json::Value> = providers
        .iter()
        .filter(|(name, _)| !deleted.contains(name))
        .map(|(name, item)| admin_provider_row(name, item))
        .collect();
    axum::Json(serde_json::json!({ "providers": out })).into_response()
}

/// GET /router/api/defaults：默认 provider/model 预选数据（workbench-
/// conversation-view 4.1）。路径名保留（路由面稳定），数据面随 make-core-own-
/// provider-data 的契约走：router 的 `/admin/defaults` 已是 404 下线面，
/// defaults 真源在 core 状态库（providers 域快照的 `default_selection` 段，
/// 沿 providers_snapshot 同一 seam 读）。未设置 → 双 null（语义照旧）；
/// core 不可达 → 503（不回退陈快照、不伪造默认值）。
pub async fn router_api_defaults(State(state): State<WebUiState>) -> axum::response::Response {
    let Some(snapshot) = providers_snapshot(&state).await else {
        return err_503_core_unreachable();
    };
    let selection = snapshot.get("default_selection");
    let non_empty = |v: Option<&serde_json::Value>| -> Option<String> {
        v.and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let provider = non_empty(selection.and_then(|s| s.get("provider")));
    let model = non_empty(selection.and_then(|s| s.get("model")));
    axum::Json(serde_json::json!({
        "default_provider": provider,
        "default_model": model,
    }))
    .into_response()
}

/// GET /router/api/presets：内置 preset 表只读视图（seam 直出代码表；detached
/// 形态经 core channel 的 presets 域，同一形状）。core 不可达 → 503（表本身
/// 是代码数据，但 preserve「真源可达才服务」的读姿态——快照 seam 同时是
/// core 可达性探测）。
pub async fn router_api_presets(State(state): State<WebUiState>) -> axum::response::Response {
    match state.backend.state_snapshot("presets").await {
        Some(v) if v.get("error").is_none() => axum::Json(v).into_response(),
        _ => err_503_core_unreachable(),
    }
}

/// 把 core 的 mutation 结果映射成 HTTP 响应。`ok_body` 在成功时返回
/// (status, body)；业务拒绝按 cause 映射 400/404/409；传输类失败 → 503。
fn map_mutation_result(
    result: Result<(), String>,
    ok: impl FnOnce() -> (axum::http::StatusCode, serde_json::Value),
) -> axum::response::Response {
    match result {
        Ok(()) => {
            let (status, body) = ok();
            (status, axum::Json(body)).into_response()
        }
        Err(cause) => map_mutation_error(&cause).into_response(),
    }
}

/// rejection cause → HTTP 状态（见模块头注释的状态码契约）。先判
/// 「不存在 / 已存在」（core delete 类拒绝的 cause 不带 op 前缀），再判
/// op 前缀（业务校验拒绝），都不中 = 传输类失败 → 503。
fn map_mutation_error(cause: &str) -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    if cause.contains("不存在") {
        return not_found(cause);
    }
    if cause.contains("已存在") {
        return (
            axum::http::StatusCode::CONFLICT,
            axum::Json(serde_json::json!({"error": cause})),
        );
    }
    let business = [
        "put:",
        "delete:",
        "save:",
        "set_defaults",
        "clear_defaults",
        "providers: ",
        "aliases: ",
    ]
    .iter()
    .any(|prefix| cause.contains(prefix));
    if business {
        return bad_request(cause);
    }
    // 传输类失败：core 不可达。诚实 503，不伪装、不回退。
    (
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({"error": cause})),
    )
}

fn bad_request(cause: &str) -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    (
        axum::http::StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({"error": cause})),
    )
}

fn not_found(cause: &str) -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    (
        axum::http::StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({"error": cause})),
    )
}

/// core 不可达的统一 503（读路径）。
fn err_503_core_unreachable() -> axum::response::Response {
    (
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({"error": "core 状态库不可达：provider 数据真源离线，拒绝返回可能过期的快照"})),
    )
        .into_response()
}

/// 统一 409（重名；store 快照预检）。
fn conflict(msg: impl Into<String>) -> axum::response::Response {
    (
        axum::http::StatusCode::CONFLICT,
        axum::Json(serde_json::json!({"error": msg.into()})),
    )
        .into_response()
}

/// 快照里 store 段的 provider 条目（幂等取 Map）。
fn stored_providers(snapshot: &serde_json::Value) -> &serde_json::Map<String, serde_json::Value> {
    static EMPTY: std::sync::OnceLock<serde_json::Map<String, serde_json::Value>> =
        std::sync::OnceLock::new();
    snapshot
        .get("providers")
        .and_then(serde_json::Value::as_object)
        .unwrap_or_else(|| EMPTY.get_or_init(Default::default))
}

/// 条目 `models` 值 → 条目列表（admin 行投影的兼容读取）。条目对象原样
/// 保留（id + 能力标记）；遗留裸字符串 → 仅隐含 text 的条目；逗号分隔
/// 字符串 → 逐段成条目。元素非法（未知能力标记等）→ None（整段按「条目
/// 未自带目录」处理，回落 preset 代码表——display 投影不因脏数据 500）。
fn models_value_to_entries(v: &serde_json::Value) -> Option<Vec<sebas_router::models::ModelEntry>> {
    use sebas_router::models::ModelEntry;
    match v {
        serde_json::Value::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for el in arr {
                out.push(serde_json::from_value::<ModelEntry>(el.clone()).ok()?);
            }
            Some(out)
        }
        serde_json::Value::String(s) => Some(
            s.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ModelEntry::text_only)
                .collect(),
        ),
        _ => None,
    }
}

pub async fn router_api_provider_create(
    State(state): State<WebUiState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let Some(snapshot) = providers_snapshot(&state).await else {
        return err_503_core_unreachable();
    };
    let Some(name) = body
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    else {
        return bad_request("缺少 name 字段").into_response();
    };
    // 重名检查以 store 为准（旧语义：409）。
    if stored_providers(&snapshot).contains_key(&name) {
        return conflict(format!("provider '{name}' 已存在"));
    }
    let item = body;
    map_mutation_result(
        state
            .backend
            .state_mutate(
                "providers",
                serde_json::json!({"op": "put", "name": name, "item": item}),
            )
            .await,
        || {
            (
                axum::http::StatusCode::CREATED,
                serde_json::json!({"created": name}),
            )
        },
    )
}

pub async fn router_api_provider_update(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let Some(snapshot) = providers_snapshot(&state).await else {
        return err_503_core_unreachable();
    };
    // 未知 provider → 404（不静默 upsert）。存在性以 store 为准：config 种子
    // provider 不经 WebUI 管理（真源只有 store）。
    if !stored_providers(&snapshot).contains_key(&name) {
        return not_found(&format!(
            "provider '{name}' 不存在（store 无此条目；config 种子 provider 不经 WebUI 管理）"
        ))
        .into_response();
    }
    // 合并旧值：空/缺 api_key → 保留 store 里的 key 材料（旧 admin 语义）。
    let mut item = body;
    let empty_key = item
        .get("api_key")
        .and_then(serde_json::Value::as_str)
        .map(str::is_empty)
        .unwrap_or(true);
    if empty_key
        && let Some(old_key) = stored_providers(&snapshot)
            .get(&name)
            .and_then(|old| old.get("api_key"))
        && let Some(obj) = item.as_object_mut()
    {
        obj.insert("api_key".into(), old_key.clone());
    }
    map_mutation_result(
        state
            .backend
            .state_mutate(
                "providers",
                serde_json::json!({"op": "put", "name": name, "item": item}),
            )
            .await,
        || {
            (
                axum::http::StatusCode::OK,
                serde_json::json!({"updated": name}),
            )
        },
    )
}

pub async fn router_api_provider_delete(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> axum::response::Response {
    if providers_snapshot(&state).await.is_none() {
        return err_503_core_unreachable();
    }
    map_mutation_result(
        state
            .backend
            .state_mutate(
                "providers",
                serde_json::json!({"op": "delete", "name": name}),
            )
            .await,
        || {
            (
                axum::http::StatusCode::OK,
                serde_json::json!({"deleted": name}),
            )
        },
    )
}

/// POST /router/api/providers/{name}/probe：上游 model 列表抓取（add-fetch-models）。
/// 由 core 的 providers 域抓取 op 承载（seam 直调内嵌引擎，detached 形态经
/// core channel 同一 op）。**只读**：抓取不改 provider 任何字段、不持久化；
/// 选中某个 id 是后续的普通编辑（PUT）。响应只含 id 列表，绝无 key 材料。
pub async fn router_api_provider_probe(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> axum::response::Response {
    match state.backend.fetch_provider_models(&name).await {
        Ok(models) => {
            axum::Json(serde_json::json!({ "provider": name, "models": models })).into_response()
        }
        // rejection cause 契约（providers_fetch_models）：`fetch_models: ` 前缀
        // = core 的业务级拒绝——不存在 → 404，未配置 base url → 400，上游失败
        // → 502（坏网关，如实）；其余（store 未初始化 / 通道不可达）→ 503。
        Err(cause) => {
            let sanitized = cause
                .strip_prefix("state mutation rejected: ")
                .unwrap_or(&cause);
            if sanitized.contains("state store 未初始化")
                || sanitized.contains("core 不可达")
                || sanitized.contains("core providers 域未由此后端承载")
            {
                return err_503_core_unreachable();
            }
            if sanitized.contains("不存在") {
                return not_found(sanitized).into_response();
            }
            if sanitized.contains("未配置任何 base URL") {
                return bad_request(sanitized).into_response();
            }
            if sanitized.starts_with("fetch_models:") {
                return (
                    axum::http::StatusCode::BAD_GATEWAY,
                    axum::Json(serde_json::json!({"error": sanitized})),
                )
                    .into_response();
            }
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({"error": sanitized})),
            )
                .into_response()
        }
    }
}

pub async fn router_api_alias_create(
    State(state): State<WebUiState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let Some(snapshot) = providers_snapshot(&state).await else {
        return err_503_core_unreachable();
    };
    let (alias, entry) = match validated_alias(&snapshot, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    if snapshot
        .get("model_aliases")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|m| m.contains_key(&alias))
    {
        return conflict(format!("别名 '{alias}' 已存在"));
    }
    map_mutation_result(
        state
            .backend
            .state_mutate(
                "aliases",
                serde_json::json!({"op": "put", "alias": alias, "entry": entry}),
            )
            .await,
        || {
            (
                axum::http::StatusCode::CREATED,
                serde_json::json!({"created": alias}),
            )
        },
    )
}

pub async fn router_api_alias_update(
    State(state): State<WebUiState>,
    Path(alias): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let Some(snapshot) = providers_snapshot(&state).await else {
        return err_503_core_unreachable();
    };
    let (_alias, entry) = match validated_alias(&snapshot, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    if !snapshot
        .get("model_aliases")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|m| m.contains_key(&alias))
    {
        return not_found(&format!("别名 '{alias}' 不存在")).into_response();
    }
    map_mutation_result(
        state
            .backend
            .state_mutate(
                "aliases",
                serde_json::json!({"op": "put", "alias": alias, "entry": entry}),
            )
            .await,
        || {
            (
                axum::http::StatusCode::OK,
                serde_json::json!({"updated": alias}),
            )
        },
    )
}

pub async fn router_api_alias_delete(
    State(state): State<WebUiState>,
    Path(alias): Path<String>,
) -> axum::response::Response {
    if providers_snapshot(&state).await.is_none() {
        return err_503_core_unreachable();
    }
    map_mutation_result(
        state
            .backend
            .state_mutate(
                "aliases",
                serde_json::json!({"op": "delete", "alias": alias}),
            )
            .await,
        || {
            (
                axum::http::StatusCode::OK,
                serde_json::json!({"deleted": alias}),
            )
        },
    )
}

/// 别名 body 校验（与旧 router admin 同一规则：非空、无 `/`、无 `*`、
/// provider 存在于 store）。返回 (alias, entry wire)。
fn validated_alias(
    snapshot: &serde_json::Value,
    body: &serde_json::Value,
) -> Result<(String, serde_json::Value), Box<axum::response::Response>> {
    let alias = body
        .get("alias")
        .or_else(|| body.get("name"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("缺少 alias 字段").into_response())?;
    if alias.contains('/') {
        return Err(Box::new(
            bad_request("alias 不能包含 '/'（保留给命名空间语法）").into_response(),
        ));
    }
    // 别名只精确匹配、不参与 glob（router-model-aliases spec）。
    if alias.contains('*') {
        return Err(Box::new(
            bad_request("alias 不能包含 '*'（别名只精确匹配，不支持 glob）").into_response(),
        ));
    }
    let provider = body
        .get("provider")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("缺少 provider 字段").into_response())?;
    if !stored_providers(snapshot).contains_key(provider) {
        return Err(Box::new(
            bad_request(&format!("provider '{provider}' 不存在")).into_response(),
        ));
    }
    let mut entry = serde_json::Map::new();
    entry.insert(
        "provider".into(),
        serde_json::Value::String(provider.to_string()),
    );
    if let Some(up) = body
        .get("upstream_model")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
    {
        entry.insert(
            "upstream_model".into(),
            serde_json::Value::String(up.to_string()),
        );
    }
    Ok((alias.to_string(), serde_json::Value::Object(entry)))
}

/// POST /router/api/reload：router 配置刷新代理（读侧操作，仍走 router 的
/// `/admin/reload`——它不写 provider 数据，只是让 router 重投影 core 快照 /
/// 文件 overlay）。
pub async fn router_api_reload(State(state): State<WebUiState>) -> axum::response::Response {
    let client = router_client_of(&state);
    if state.router.listen.is_none() {
        return err_503_core_unreachable();
    }
    match client.reload().await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => (
            e.status,
            axum::Json(serde_json::json!({"error": e.message})),
        )
            .into_response(),
    }
}

/// router mutation 守卫（Task 6.3，语义与 admin_mutation_guard 一致但
/// 不依赖 AdminState）：POST-only（405）+ loopback origin 检查（403）。
pub async fn router_mutation_guard(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::Method;
    // 只放行变更语义（POST/PUT/DELETE）；GET/HEAD 等读方法 405。
    if !matches!(req.method(), &Method::POST | &Method::PUT | &Method::DELETE) {
        return (axum::http::StatusCode::METHOD_NOT_ALLOWED, "mutation only").into_response();
    }
    let origin_ok = req
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .map(|o| {
            o.is_empty()
                || o.strip_prefix("http://")
                    .map(|rest| {
                        let host = rest.split(':').next().unwrap_or(rest);
                        host == "127.0.0.1" || host == "localhost" || host == "::1"
                    })
                    .unwrap_or(false)
        })
        .unwrap_or(true); // 无 origin（CLI/curl）放行——router 侧另有 bearer 鉴权
    if !origin_ok {
        return (axum::http::StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    next.run(req).await
}

/// Health probe: `GET /health`.
pub async fn health() -> &'static str {
    "ok\n"
}
