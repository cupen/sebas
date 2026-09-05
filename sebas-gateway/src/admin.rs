//! Admin HTTP 面（change gateway-admin-api-and-model-aliases，design D5）。
//!
//! - `/admin/*`：providers / model-aliases CRUD、probe、reload、stats。
//! - 鉴权独立于透传流量：`SEBAS_CONTROL_SECRET` Bearer；无 secret 时仅
//!   loopback 放行（standalone 模式，启动 warn）。
//! - 挂在主 router 的 proxy fallback 之上，不受 require_key/rate_limit 影响。
//!
//! 写路径（design D3）：校验先行（400 拒绝不碰文件）→ Map 级 RMW（只动
//! 目标段，保留其它 key）→ tempfile+rename 原子落盘 → `swap_core` 热替换。
//! 与 router 卡片写路径（state_store）共用 providers.json，双写者共存。

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Map, Value};

use crate::config::{self, GatewayConfig};
use crate::server::AppState;

/// 通用 401 文案（与 auth.rs 同款铁律：不回显呈现的 token）。
const UNAUTHORIZED_MSG: &str = "invalid or missing admin credentials";

/// Admin router：`/admin/*` 全部端点。挂载方须把它 nest 在 proxy fallback
/// 之上并套 `admin_auth`（本模块提供 middleware，装配见 `build_admin_router`）。
pub fn build_admin_router(state: AppState) -> Router {
    Router::new()
        .route("/admin/providers", get(list_providers).post(create_provider))
        .route(
            "/admin/providers/{name}",
            axum::routing::put(update_provider).delete(delete_provider),
        )
        .route("/admin/providers/{name}/probe", post(probe_provider))
        .route(
            "/admin/model-aliases",
            get(list_aliases).post(create_alias),
        )
        .route(
            "/admin/model-aliases/{alias}",
            axum::routing::put(update_alias).delete(delete_alias),
        )
        .route("/admin/reload", post(reload))
        .route("/admin/stats", get(stats))
        .route("/metrics", get(metrics))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            admin_auth,
        ))
        .with_state(state)
}

/// Admin 鉴权中间件（spec Admin authentication）：
/// - `SEBAS_CONTROL_SECRET` 非空 → 校验 `Authorization: Bearer <secret>`；
/// - 无 secret → 仅 loopback 客户端放行（standalone），否则 401；
///   该模式下 gateway 启动时 warn 一次（`warn_no_secret_once`）。
///
/// 401 message 恒为通用串。
pub async fn admin_auth(
    State(_state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    req: Request,
    next: Next,
) -> Response {
    let secret = std::env::var("SEBAS_CONTROL_SECRET").unwrap_or_default();
    let ok = if !secret.is_empty() {
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .is_some_and(|t| constant_time_eq(t, &secret))
    } else {
        addr.ip().is_loopback()
    };
    if !ok {
        return (StatusCode::UNAUTHORIZED, UNAUTHORIZED_MSG).into_response();
    }
    next.run(req).await
}

/// 手写常量时间比较（timing-safe，无需新依赖）。
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 写路径统一入口（5.3 通道代理）：core channel 可用时把 provider/alias
/// 变更写到 core 状态库，否则回退 overlay 文件。
/// 返回 `Ok(true)` = 走了通道；`Ok(false)` = 走了文件回退。
/// 调用方在 Ok 后继续走统一的 reload（`reload_and_swap` 或通道快照投影）。
pub(crate) async fn channel_write(
    domain: &str,
    payload: serde_json::Value,
) -> Result<bool, String> {
    let Some(socket) = crate::core_channel::socket_path() else {
        return Ok(false);
    };
    match crate::core_channel::mutate_state(&socket, domain, payload).await {
        Ok(()) => Ok(true),
        Err(e) => {
            tracing::warn!(error = %e, domain = %domain, "core channel mutation 失败, 回退文件写路径");
            Ok(false)
        }
    }
}

/// 写后的统一 reload（async）：
/// - 通道写成功 → 立即用通道快照投影重建配置（响应前生效）。
/// - 文件写 / 通道不可用 → 走 `reload_and_swap`（文件 overlay 重读）。
pub(crate) async fn reload_after_write(state: &AppState) -> Result<(), String> {
    if let Some(socket) = crate::core_channel::socket_path() {
        // 同步拉一次快照投影（不等订阅广播，保证响应已含新配置）。
        crate::core_channel::reload_from_channel(state, &socket).await;
        // 投影成功会 record_source_ok + record_ok_quiet；失败已 record_err。
        // 这里以投影是否成功为准返回。
        if state.reload_status.error().is_none() && state.reload_status.source_unavailable().is_none()
        {
            return Ok(());
        }
        tracing::info!("通道投影未生效，回退文件重载");
    }
    crate::admin::reload_and_swap(state)
}

/// 启动时调用：standalone 无 secret 模式 warn 一次。
pub fn warn_no_secret_once() {
    let secret = std::env::var("SEBAS_CONTROL_SECRET").unwrap_or_default();
    if secret.is_empty() {
        tracing::warn!(
            "[gateway] SEBAS_CONTROL_SECRET 未设置：admin 面仅接受 loopback 连接"
        );
    }
}

// -------------------- overlay RMW 底座 --------------------

/// providers.json 路径（与 config.rs 的 `provider_overlay` 同源；此处从
/// 当前内核 cfg 取——env 覆盖已在 parse 时应用）。
fn overlay_path(state: &AppState) -> PathBuf {
    PathBuf::from(state.core().cfg.provider_overlay.clone())
}

/// 读 providers.json 原始 Map。缺失 → 空 Map；解析失败 → Err（admin 面对
/// 破损文件显式报错，区别于启动路径的保旧）。
fn read_overlay_raw(path: &std::path::Path) -> Result<Map<String, Value>, String> {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str::<Value>(&raw)
            .map_err(|e| format!("providers.json 解析失败: {e}"))?
            .as_object()
            .cloned()
            .ok_or_else(|| "providers.json 顶层不是对象".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(format!("providers.json 读取失败: {e}")),
    }
}

/// Map 级 RMW 原子写：读现有 Map（保留未知段）→ 应用闭包改写 → tempfile +
/// rename。闭包返回 Err 时不动文件。
fn write_overlay_rmw<F>(path: &std::path::Path, f: F) -> Result<(), String>
where
    F: FnOnce(&mut Map<String, Value>) -> Result<(), String>,
{
    let mut root = read_overlay_raw(path)?;
    f(&mut root)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let body = serde_json::to_string_pretty(&Value::Object(root))
        .map_err(|e| format!("序列化失败: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("写临时文件失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename 失败: {e}"))?;
    Ok(())
}

/// 写后热替换：重读配置（config.toml 种子 + 新 overlay）→ build 校验 →
/// swap_core。admin 写路径专用：成功后记录当前 overlay 内容（watcher 见
/// 到相同内容时跳过 reload，不重复消费同一变更）并清 last_reload_error；
/// 失败保旧内核，错误记入 reload_status（供 stats；文件已持久，下次有效
/// 写自动恢复）。
pub(crate) fn reload_and_swap(state: &AppState) -> Result<(), String> {
    let res = reload_and_swap_inner(state);
    match &res {
        Ok(()) => {
            let path = overlay_path(state);
            if let Ok(content) = std::fs::read_to_string(&path) {
                state.reload_status.mark_admin_write(&content);
            }
            state.reload_status.record_ok_quiet();
        }
        Err(e) => state.reload_status.record_err(e),
    }
    res
}

fn reload_and_swap_inner(state: &AppState) -> Result<(), String> {
    let cfg = rebuild_from_seed(state)?;
    state
        .swap_core(cfg)
        .map_err(|e| format!("热替换失败: {e}"))
}

/// 从 config.toml 种子重建 GatewayConfig，保留外壳启动期字段。
/// 不含 overlay 合并（调用方决定数据源：文件 or core channel 快照）。
pub(crate) fn rebuild_from_seed(state: &AppState) -> Result<GatewayConfig, String> {
    let core = state.core();
    let toml_path = &core.cfg.config_source;
    let raw_toml = std::fs::read_to_string(toml_path)
        .map_err(|e| format!("读 config.toml ({toml_path}) 失败: {e}"))?;
    let mut cfg = GatewayConfig::parse(&raw_toml).map_err(|e| format!("解析失败: {e}"))?;
    // 保留外壳的启动期字段（listen/超时等不因 reload 变化——它们来自原
    // cfg 而非新读）。
    cfg.listen = core.cfg.listen.clone();
    cfg.max_body_bytes = core.cfg.max_body_bytes;
    cfg.connect_timeout_secs = core.cfg.connect_timeout_secs;
    cfg.read_timeout_secs = core.cfg.read_timeout_secs;
    cfg.usage_file = core.cfg.usage_file.clone();
    cfg.debug = core.cfg.debug;
    cfg.rate_limit = core.cfg.rate_limit;
    Ok(cfg)
}

// -------------------- providers CRUD --------------------

/// GET /admin/providers：全 provider 列表，key 脱敏（api_key_configured bool）。
async fn list_providers(State(state): State<AppState>) -> Response {
    let core = state.core();
    let mut out = Vec::new();
    for (name, p) in &core.cfg.providers {
        out.push(json!({
            "name": name,
            "base_url_anthropic": p.base_url_anthropic,
            "base_url_openai": p.base_url_openai,
            "api_key_env": p.api_key_env,
            "api_key_configured": core.api_keys.contains_key(name),
            "models": p.models,
        }));
    }
    Json(json!({ "providers": out })).into_response()
}

/// POST /admin/providers：创建。重名 409；无效 400（不碰文件）。
async fn create_provider(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Response {
    let Some(name) = body.get("name").and_then(Value::as_str).map(str::to_string) else {
        return err_400("缺少 name 字段");
    };
    let Some(item) = body.as_object() else {
        return err_400("body 必须是对象");
    };
    // 校验先行：候选在内存完整跑 resolve 管线。
    if let Err(e) = config::validate_provider_entry(&name, item) {
        return err_400(&e.to_string());
    }
    // 重名检查（overlay + 种子）。
    let core = state.core();
    if core.cfg.providers.contains_key(&name) {
        return err_409(&format!("provider '{name}' 已存在"));
    }
    // 5.3 通道代理：core channel 可用时写状态库；否则回退 overlay 文件。
    let item_value = item.clone();
    let name2 = name.clone();
    let via_channel = match channel_write(
        "providers",
        json!({"op": "put", "name": name2.clone(), "item": item_value.clone()}),
    )
    .await
    {
        Ok(true) => true,
        Ok(false) => false,
        Err(e) => return err_500(&e),
    };
    if !via_channel {
        let path = overlay_path(&state);
        if let Err(e) = write_overlay_rmw(&path, |root| {
            let providers = root
                .entry("providers".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            let Some(map) = providers.as_object_mut() else {
                return Err("providers 段损坏（非对象）".into());
            };
            if map.contains_key(&name2) {
                return Err(format!("provider '{name2}' 已存在"));
            }
            map.insert(name2.clone(), Value::Object(item_value.clone()));
            // 创建即撤销墓碑。
            if let Some(deleted) = root.get_mut("deleted").and_then(Value::as_array_mut) {
                deleted.retain(|d| d.as_str() != Some(name2.as_str()));
            }
            Ok(())
        }) {
            return err_500(&e);
        }
    }
    match reload_after_write(&state).await {
        Ok(()) => (StatusCode::CREATED, Json(json!({"created": name}))).into_response(),
        Err(e) => err_500(&e),
    }
}

/// PUT /admin/providers/{name}：更新。空/缺 api_key 保留旧值；未知 404。
async fn update_provider(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let path = overlay_path(&state);
    let Some(item) = body.as_object() else {
        return err_400("body 必须是对象");
    };
    // 合并旧值：空/缺 api_key → 保留旧条目的 key 材料。
    let merged: Map<String, Value> = {
        let root = match read_overlay_raw(&path) {
            Ok(r) => r,
            Err(e) => return err_500(&e),
        };
        let old = root
            .get("providers")
            .and_then(Value::as_object)
            .and_then(|m| m.get(&name));
        match old {
            Some(Value::Object(old)) => {
                let mut m = old.clone();
                for (k, v) in item {
                    let keep_old = k == "api_key"
                        && v.as_str().map(str::is_empty).unwrap_or(true);
                    if !keep_old {
                        m.insert(k.clone(), v.clone());
                    }
                }
                m
            }
            // 不在 overlay（None）或条目非对象：当作全新条目处理——未知
            // 与否留给 swap 校验。
            _ => item.clone(),
        }
    };
    if let Err(e) = config::validate_provider_entry(&name, &merged) {
        return err_400(&e.to_string());
    }
    let merged_value = Value::Object(merged);
    let name2 = name.clone();
    // 5.3 通道代理：优先写状态库（put 全量覆盖）；不可用回退文件。
    let via_channel = match channel_write(
        "providers",
        json!({"op": "put", "name": name2.clone(), "item": merged_value.clone()}),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return err_500(&e),
    };
    if !via_channel
        && let Err(e) = write_overlay_rmw(&path, |root| {
            let providers = root
                .entry("providers".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            let Some(map) = providers.as_object_mut() else {
                return Err("providers 段损坏（非对象）".into());
            };
            map.insert(name2.clone(), merged_value.clone());
            if let Some(deleted) = root.get_mut("deleted").and_then(Value::as_array_mut) {
                deleted.retain(|d| d.as_str() != Some(name2.as_str()));
            }
            Ok(())
        })
    {
        return err_500(&e);
    }
    match reload_after_write(&state).await {
        Ok(()) => (StatusCode::OK, Json(json!({"updated": name}))).into_response(),
        Err(e) => err_500(&e),
    }
}

/// DELETE /admin/providers/{name}：overlay 条目直接删；config 种子来源的
/// 写墓碑（跨重启生效）。未知 404。
async fn delete_provider(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    let path = overlay_path(&state);
    let core = state.core();
    let in_seed = core.cfg.providers.contains_key(&name);
    let root = match read_overlay_raw(&path) {
        Ok(r) => r,
        Err(e) => return err_500(&e),
    };
    let in_overlay = root
        .get("providers")
        .and_then(Value::as_object)
        .is_some_and(|m| m.contains_key(&name));
    if !in_seed && !in_overlay {
        // 通道模式：provider 可能在状态库而非 overlay/种子（卡片写入）。
        let socket_available = crate::core_channel::socket_path().is_some();
        if socket_available {
            let via_channel = match channel_write(
                "providers",
                json!({"op": "delete", "name": name.clone()}),
            )
            .await
            {
                Ok(true) => true,
                Ok(false) => false,
                Err(e) => {
                    // 状态库说 provider 不存在 → 404。
                    if e.contains("不存在") {
                        return err_404(&format!("provider '{name}' 不存在"));
                    }
                    return err_500(&e);
                }
            };
            if via_channel {
                return match reload_after_write(&state).await {
                    Ok(()) => (StatusCode::OK, Json(json!({"deleted": name}))).into_response(),
                    Err(e) => err_500(&e),
                };
            }
        }
        return err_404(&format!("provider '{name}' 不存在"));
    }
    let name2 = name.clone();
    let seed_sourced = in_seed && !in_overlay;
    // 5.3 通道代理优先：读到了（seed/overlay）但也许状态库是另一套——
    // 统一写状态库（含墓碑），失败回退文件。
    let via_channel = match channel_write(
        "providers",
        json!({"op": "delete", "name": name2.clone()}),
    )
    .await
    {
        Ok(true) => {
            if let Err(e) = reload_after_write(&state).await {
                return err_500(&e);
            }
            return (StatusCode::OK, Json(json!({"deleted": name}))).into_response();
        }
        Ok(false) => false,
        Err(e) => return err_500(&e),
    };
    let _ = via_channel;
    let _ = seed_sourced;
    if let Err(e) = write_overlay_rmw(&path, |root| {
        if let Some(map) = root.get_mut("providers").and_then(Value::as_object_mut) {
            map.remove(&name2);
        }
        if seed_sourced {
            let deleted = root
                .entry("deleted".to_string())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Some(arr) = deleted.as_array_mut()
                && !arr.iter().any(|d| d.as_str() == Some(name2.as_str()))
            {
                arr.push(Value::String(name2.clone()));
            }
        }
        Ok(())
    }) {
        return err_500(&e);
    }
    match reload_after_write(&state).await {
        Ok(()) => (StatusCode::OK, Json(json!({"deleted": name}))).into_response(),
        Err(e) => err_500(&e),
    }
}

// -------------------- model aliases CRUD --------------------

/// GET /admin/model-aliases。
async fn list_aliases(State(state): State<AppState>) -> Response {
    let core = state.core();
    let aliases: BTreeMap<String, Value> = core
        .cfg
        .model_aliases
        .iter()
        .map(|(alias, up)| {
            let mut m = Map::new();
            // provider 名需要从 routes 反查（编译后 alias→RouteGroup）。
            let provider = core
                .cfg
                .routes
                .iter()
                .find(|r| r.model == *alias)
                .and_then(|r| r.providers.first().cloned());
            if let Some(p) = provider {
                m.insert("provider".into(), Value::String(p));
            }
            if let Some(u) = up {
                m.insert("upstream_model".into(), Value::String(u.clone()));
            }
            (alias.clone(), Value::Object(m))
        })
        .collect();
    Json(json!({ "model_aliases": aliases })).into_response()
}

/// POST /admin/model-aliases：创建。校验：非空、无 `/`、provider 存在；
/// 重名 409。
async fn create_alias(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let (alias, entry) = match parse_alias_body(&state, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    let path = overlay_path(&state);
    if alias_exists(&path, &alias) {
        return err_409(&format!("别名 '{alias}' 已存在"));
    }
    let entry_value = entry.clone();
    let alias2 = alias.clone();
    // 5.3 通道代理：aliases 域写状态库；不可用回退文件。
    let via_channel = match channel_write(
        "aliases",
        json!({"op": "put", "alias": alias2.clone(), "entry": entry_value.clone()}),
    )
    .await
    {
        Ok(true) => true,
        Ok(false) => false,
        Err(e) => return err_500(&e),
    };
    if !via_channel
        && let Err(e) = write_overlay_rmw(&path, |root| {
            let aliases = root
                .entry("model_aliases".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            let Some(map) = aliases.as_object_mut() else {
                return Err("model_aliases 段损坏（非对象）".into());
            };
            map.insert(alias2.clone(), entry_value.clone());
            Ok(())
        })
    {
        return err_500(&e);
    }
    match reload_after_write(&state).await {
        Ok(()) => (StatusCode::CREATED, Json(json!({"created": alias}))).into_response(),
        Err(e) => err_500(&e),
    }
}

/// PUT /admin/model-aliases/{alias}：更新。未知 404。
async fn update_alias(
    State(state): State<AppState>,
    axum::extract::Path(alias): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let (_alias, entry) = match parse_alias_body(&state, &body) {
        Ok(v) => v,
        Err(resp) => return *resp,
    };
    let path = overlay_path(&state);
    if !alias_exists(&path, &alias) {
        return err_404(&format!("别名 '{alias}' 不存在"));
    }
    let entry_value = entry.clone();
    let alias2 = alias.clone();
    // 5.3 通道代理。
    let via_channel = match channel_write(
        "aliases",
        json!({"op": "put", "alias": alias2.clone(), "entry": entry_value.clone()}),
    )
    .await
    {
        Ok(true) => true,
        Ok(false) => false,
        Err(e) => return err_500(&e),
    };
    if !via_channel
        && let Err(e) = write_overlay_rmw(&path, |root| {
            let aliases = root
                .entry("model_aliases".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            let Some(map) = aliases.as_object_mut() else {
                return Err("model_aliases 段损坏（非对象）".into());
            };
            map.insert(alias2.clone(), entry_value.clone());
            Ok(())
        })
    {
        return err_500(&e);
    }
    match reload_after_write(&state).await {
        Ok(()) => (StatusCode::OK, Json(json!({"updated": alias}))).into_response(),
        Err(e) => err_500(&e),
    }
}

/// DELETE /admin/model-aliases/{alias}。
async fn delete_alias(
    State(state): State<AppState>,
    axum::extract::Path(alias): axum::extract::Path<String>,
) -> Response {
    let path = overlay_path(&state);
    if !alias_exists(&path, &alias) {
        return err_404(&format!("别名 '{alias}' 不存在"));
    }
    let alias2 = alias.clone();
    // 5.3 通道代理：aliases 域写状态库；不可用回退文件。
    let via_channel = match channel_write("aliases", json!({"op": "delete", "alias": alias2.clone()}))
        .await
    {
        Ok(true) => true,
        Ok(false) => false,
        Err(e) => return err_500(&e),
    };
    if !via_channel
        && let Err(e) = write_overlay_rmw(&path, |root| {
            if let Some(map) = root
                .get_mut("model_aliases")
                .and_then(Value::as_object_mut)
            {
                map.remove(&alias2);
            }
            Ok(())
        })
    {
        return err_500(&e);
    }
    match reload_after_write(&state).await {
        Ok(()) => (StatusCode::OK, Json(json!({"deleted": alias}))).into_response(),
        Err(e) => err_500(&e),
    }
}

/// 校验别名 body：非空、无 `/`、provider 存在。返回 (alias, entry wire)。
fn parse_alias_body(
    state: &AppState,
    body: &Value,
) -> Result<(String, Value), Box<Response>> {
    let alias = body
        .get("alias")
        .or_else(|| body.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Box::new(err_400("缺少 alias 字段")))?;
    if alias.is_empty() {
        return Err(Box::new(err_400("alias 不能为空")));
    }
    if alias.contains('/') {
        return Err(Box::new(err_400("alias 不能包含 '/'（保留给命名空间语法）")));
    }
    let provider = body
        .get("provider")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Box::new(err_400("缺少 provider 字段")))?;
    let core = state.core();
    if !core.cfg.providers.contains_key(&provider) {
        return Err(Box::new(err_400(&format!("provider '{provider}' 不存在"))));
    }
    let mut entry = Map::new();
    entry.insert("provider".into(), Value::String(provider));
    if let Some(up) = body.get("upstream_model").and_then(Value::as_str)
        && !up.is_empty()
    {
        entry.insert("upstream_model".into(), Value::String(up.to_string()));
    }
    Ok((alias, Value::Object(entry)))
}

fn alias_exists(path: &std::path::Path, alias: &str) -> bool {
    read_overlay_raw(path)
        .ok()
        .and_then(|r| {
            r.get("model_aliases")
                .and_then(Value::as_object)
                .map(|m| m.contains_key(alias))
        })
        .unwrap_or(false)
}

// -------------------- probe / reload / stats / metrics --------------------

/// POST /admin/providers/{name}/probe：OpenAI `/models` 优先、Anthropic
/// `/v1/models` 回退。`?apply=true` 时把列表写回 provider `models` 字段。
async fn probe_provider(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> Response {
    let core = state.core();
    let Some(p) = core.cfg.providers.get(&name) else {
        return err_404(&format!("provider '{name}' 不存在"));
    };
    let key = core.api_keys.get(&name).cloned();
    let (openai_base, anthropic_base) =
        (p.base_url_openai.clone(), p.base_url_anthropic.clone());
    if openai_base.is_none() && anthropic_base.is_none() {
        return err_400(&format!("provider '{name}' 未配置任何 base URL"));
    }
    let client = state.client.clone();
    // 依次尝试：OpenAI /models → Anthropic /v1/models。
    let mut models: Option<Vec<String>> = None;
    let mut last_err = String::new();
    if let Some(base) = &openai_base {
        match fetch_models(&client, &format!("{base}/models"), key.as_deref()).await {
            Ok(list) => models = Some(list),
            Err(e) => last_err = e,
        }
    }
    if models.is_none()
        && let Some(base) = &anthropic_base
    {
        match fetch_models(&client, &format!("{base}/v1/models"), key.as_deref()).await {
            Ok(list) => models = Some(list),
            Err(e) => last_err = e,
        }
    }
    let Some(models) = models else {
        // 502 通用 message，不含 key（last_err 已是脱敏管线产物）。
        return err_502(&format!("上游 model 列表探测失败: {last_err}"));
    };
    // ?apply=true：写回 provider 的 models 字段。
    let applied = query.as_deref() == Some("apply=true");
    if applied {
        let models_value = serde_json::to_value(&models).unwrap_or(Value::Array(Vec::new()));
        let name2 = name.clone();
        // 5.3 通道代理：从状态库快照读当前条目 → 补 models → put。
        // 通道不可用时回退文件（保留 seed 字段合并逻辑）。
        let via_channel = match crate::core_channel::socket_path() {
            Some(socket) => {
                let item =
                    // 拉 providers 快照 → 该 provider 条目。
                    match crate::core_channel::fetch_state_snapshot(&socket, "providers").await {
                        Ok(snap) => {
                            let providers = snap
                                .get("providers")
                                .and_then(Value::as_object)
                                .cloned()
                                .unwrap_or_default();
                            let mut item = providers
                                .get(&name2)
                                .cloned()
                                .unwrap_or_else(|| Value::Object(Map::new()));
                            // 若状态库无此条目（seed 来源），用当前内核的 cfg 条目补连接字段。
                            if let Some(obj) = item.as_object_mut() {
                                if obj.is_empty()
                                    && let Some(seed) = state.core().cfg.providers.get(&name2)
                                {
                                    if let Some(v) = &seed.base_url_anthropic {
                                        obj.insert("base_url_anthropic".into(), Value::String(v.clone()));
                                    }
                                    if let Some(v) = &seed.base_url_openai {
                                        obj.insert("base_url_openai".into(), Value::String(v.clone()));
                                    }
                                    if let Some(v) = &seed.api_key_env {
                                        obj.insert("api_key_env".into(), Value::String(v.clone()));
                                    }
                                    if let Some(v) = &seed.api_key {
                                        obj.insert("api_key".into(), Value::String(v.clone()));
                                    }
                                }
                                obj.insert("models".into(), models_value.clone());
                            }
                            item
                        }
                        Err(_) => Value::Null,
                    };
                if item.is_null() {
                    Ok(false)
                } else {
                    match channel_write(
                        "providers",
                        json!({"op": "put", "name": name2.clone(), "item": item}),
                    )
                    .await
                    {
                        Ok(true) => Ok(true),
                        Ok(false) => Ok(false),
                        Err(e) => Err(e),
                    }
                }
            }
            None => Ok(false),
        };
        if let Err(e) = via_channel {
            return err_500(&e);
        }
        let via_channel = via_channel.unwrap_or(false);
        if !via_channel {
            let path = overlay_path(&state);
            if let Err(e) = write_overlay_rmw(&path, |root| {
                let providers = root
                    .entry("providers".to_string())
                    .or_insert_with(|| Value::Object(Map::new()));
                let Some(map) = providers.as_object_mut() else {
                    return Err("providers 段损坏（非对象）".into());
                };
                let entry = map
                    .entry(name2.clone())
                    .or_insert_with(|| Value::Object(Map::new()));
                let Some(obj) = entry.as_object_mut() else {
                    return Err(format!("provider '{name2}' 条目非对象"));
                };
                // overlay 条目整体替换种子条目——若本 provider 原本只在
                // config.toml 里（overlay 无条目），只写 models 会抹掉
                // base_url/preset 导致校验失败。seed 里带过的连接字段须一并
                // 带入 overlay 条目。
                if obj.is_empty()
                    && let Some(seed) = state
                        .core()
                        .cfg
                        .providers
                        .get(&name2)
                {
                    if let Some(v) = &seed.base_url_anthropic {
                        obj.insert("base_url_anthropic".into(), Value::String(v.clone()));
                    }
                    if let Some(v) = &seed.base_url_openai {
                        obj.insert("base_url_openai".into(), Value::String(v.clone()));
                    }
                    if let Some(v) = &seed.api_key_env {
                        obj.insert("api_key_env".into(), Value::String(v.clone()));
                    }
                    if let Some(v) = &seed.api_key {
                        obj.insert("api_key".into(), Value::String(v.clone()));
                    }
                }
                obj.insert("models".into(), models_value.clone());
                Ok(())
            }) {
                return err_500(&e);
            }
        }
        if let Err(e) = reload_after_write(&state).await {
            return err_500(&e);
        }
    }
    Json(json!({"models": models, "applied": applied})).into_response()
}

/// GET 上游 model 列表（OpenAI `data[].id` / Anthropic `data[].id` 两种形状）。
/// 错误串脱敏：只含状态码/类别，不含 key 与 body。
async fn fetch_models(
    client: &reqwest::Client,
    url: &str,
    key: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut req = client.get(url);
    if let Some(k) = key {
        req = req.bearer_auth(k);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("连接失败: {}", e.status().map(|s| s.to_string()).unwrap_or_default()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let body: Value = resp
        .text()
        .await
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or_else(|| "响应不是 JSON".to_string())?;
    let list = body
        .get("data")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok(list)
}

/// POST /admin/reload：手动重读 + 热替换。成功返回摘要，失败返回错误文本。
async fn reload(State(state): State<AppState>) -> Response {
    match reload_and_swap(&state) {
        Ok(()) => Json(json!({"reloaded": true})).into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({"reloaded": false, "error": e})))
            .into_response(),
    }
}

/// GET /admin/stats：占位（5.3 实现 JSON 摘要；先返回最小可用结构）。
async fn stats(State(state): State<AppState>) -> Response {
    let core = state.core();
    let m = crate::metrics::Metrics::global();
    // per-provider 聚合：从 registry 的 series 名解析（requests_total /
    // upstream_errors / tokens）。registry 是进程级快照，包含全部历史
    // provider——已删除 provider 的残留计数保留（观测量，不因删除回零）。
    let mut per_provider: BTreeMap<String, Value> = BTreeMap::new();
    for (name, v) in m.snapshot() {
        let Some((labels, value)) = parse_series(&name, v) else {
            continue;
        };
        if let Some(p) = labels.get("provider") {
            let e = per_provider.entry(p.clone()).or_insert_with(|| json!({"name": p}));
            if name.starts_with("sebas_gateway_requests_total") {
                e["requests"] = value;
            } else if name.starts_with("sebas_gateway_upstream_errors_total") {
                e["errors"] = value;
            } else if name.starts_with("sebas_gateway_tokens_total") {
                match labels.get("kind").map(String::as_str) {
                    Some("input") => e["input_tokens"] = value,
                    Some("output") => e["output_tokens"] = value,
                    _ => {}
                }
            }
        }
    }
    let mut out = json!({
        "uptime_secs": m.uptime_secs(),
        "providers": core.cfg.providers.len(),
        "routes": core.cfg.routes.len(),
        "per_provider": per_provider.values().collect::<Vec<_>>(),
    });
    // 4.2：热重载状态（无失败时字段缺省——机器可读的「健康」信号）。
    if let Some(e) = state.reload_status.error() {
        out["last_reload_error"] = Value::String(e);
    }
    // 5.3：数据源（core state channel）不可用（断连时保持最后有效配置）。
    if let Some(cause) = state.reload_status.source_unavailable() {
        out["source_unavailable"] = Value::String(cause);
    }
    if let Some(t) = state.reload_status.ok_at() {
        out["last_reload_ok_at"] = Value::Number(
            t.duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().into())
                .unwrap_or(0.into()),
        );
    }
    Json(out).into_response()
}

/// 解析 series 名 → (labels, value)。非 sebas_gateway_* 前缀返回 None。
fn parse_series(name: &str, v: u64) -> Option<(BTreeMap<String, String>, Value)> {
    if !name.starts_with("sebas_gateway_") {
        return None;
    }
    let mut labels = BTreeMap::new();
    if let (Some(l), Some(r)) = (name.find('{'), name.find('}'))
        && l < r
    {
        for kv in name[l + 1..r].split(',') {
            if let Some((k, val)) = kv.split_once('=') {
                labels.insert(k.trim().to_string(), val.trim_matches('"').to_string());
            }
        }
    }
    Some((labels, Value::from(v)))
}

/// GET /metrics：手写 Prometheus 文本（0.0.4 exposition format）。
/// series 名含 label（registry 里即完整名），逐行 `<name> <value>`；非
/// ASCII 值不存在（series 名由代码生成）。附 `# HELP/TYPE` 元数据行。
async fn metrics() -> Response {
    let m = crate::metrics::Metrics::global();
    let mut out = String::new();
    for (name, _) in &m.snapshot() {
        let base = name.split('{').next().unwrap_or(name);
        if out.contains(&format!("# TYPE {base}")) {
            continue;
        }
        let mtype = if base.contains("_duration_ms_bucket") || base.contains("_duration_ms_count") {
            "histogram"
        } else if base.contains("_active_requests") {
            "gauge"
        } else {
            "counter"
        };
        out.push_str(&format!("# TYPE {base} {mtype}\n"));
    }
    for (name, v) in m.snapshot() {
        out.push_str(&format!("{name} {v}\n"));
    }
    out.push_str(&format!(
        "sebas_gateway_uptime_seconds {}\n",
        m.uptime_secs()
    ));
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        out,
    )
        .into_response()
}

// -------------------- 错误辅助 --------------------

fn err_400(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response()
}
fn err_404(msg: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
}
fn err_409(msg: &str) -> Response {
    (StatusCode::CONFLICT, Json(json!({"error": msg}))).into_response()
}
fn err_500(msg: &str) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": msg}))).into_response()
}
fn err_502(msg: &str) -> Response {
    (StatusCode::BAD_GATEWAY, Json(json!({"error": msg}))).into_response()
}
