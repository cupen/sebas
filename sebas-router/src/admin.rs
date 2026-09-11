//! Admin HTTP 面（router-admin-api；make-core-own-provider-data 2.1 起
//! 收缩为只读）。
//!
//! - 保留：`/admin/presets`（只读代码表）、`/admin/reload`、`/admin/stats`、
//!   `/metrics`、鉴权与外部热重载路径。
//! - 已下线：providers / model-aliases / defaults / probe 的全部端点——
//!   provider 数据的唯一写通道是 core 状态库（core session channel 的
//!   `providers` / `aliases` / `settings` 域），router 不再持有任何写路径。
//!   下线路由显式答 404（不是 503 桩）：路由面已不存在。
//! - 鉴权独立于透传流量：`SEBAS_CONTROL_SECRET` Bearer；无 secret 时仅
//!   loopback 放行（standalone 模式，启动 warn）。
//! - 挂在主 router 的 proxy fallback 之上，不受 require_key/rate_limit 影响。

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::config::{self, RouterConfig};
use crate::server::AppState;

/// 通用 401 文案（与 auth.rs 同款铁律：不回显呈现的 token）。
const UNAUTHORIZED_MSG: &str = "invalid or missing admin credentials";

/// 下线变更面的统一 404（design D4）：路由不存在即 404，调用方应改走 core
/// 状态库通道。显式注册而非落回 proxy fallback——否则这些路径会被当成
/// 透传请求转发到上游。
async fn route_retired() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": "route retired: provider 数据写路径已迁移至 core 状态库（core session channel）"})),
    )
        .into_response()
}

/// Admin router：`/admin/*` 全部端点。挂载方须把它 nest 在 proxy fallback
/// 之上并套 `admin_auth`（本模块提供 middleware，装配见 `build_admin_router`）。
pub fn build_admin_router(state: AppState) -> Router {
    Router::new()
        // make-core-own-provider-data 2.1：provider / alias / defaults / probe
        // 全部端点下线，显式 404（含 GET——「Provider CRUD endpoints」需求
        // 整体移除，读视图由 core 承载）。
        .route("/admin/providers", any(route_retired))
        .route("/admin/providers/{name}", any(route_retired))
        .route("/admin/providers/{name}/probe", any(route_retired))
        .route("/admin/presets", get(list_presets))
        .route("/admin/defaults", any(route_retired))
        .route("/admin/model-aliases", any(route_retired))
        .route("/admin/model-aliases/{alias}", any(route_retired))
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
///   该模式下 router 启动时 warn 一次（`warn_no_secret_once`）。
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
            .and_then(extract_bearer_token)
            .is_some_and(|t| constant_time_eq(t, &secret))
    } else {
        addr.ip().is_loopback()
    };
    if !ok {
        return (StatusCode::UNAUTHORIZED, UNAUTHORIZED_MSG).into_response();
    }
    next.run(req).await
}

/// Bearer 解析：scheme 大小写不敏感 + trim（与 auth.rs extract_key 同规则）。
fn extract_bearer_token(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.len() >= 6 && s[..6].eq_ignore_ascii_case("bearer") {
        let after = &s[6..];
        if after.starts_with(' ') || after.starts_with('\t') {
            let rest = after.trim();
            if !rest.is_empty() {
                return Some(rest);
            }
        }
    }
    None
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

/// 写后的统一 reload（async）：core channel 可用 → 立即用通道快照投影重建
/// 配置（响应前生效）；通道不可用 → `reload_and_swap`（文件 overlay 重读，
/// 无 core 的降级读路径，仍是只读操作）。
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
        tracing::info!("channel projection not applied, falling back to file reload");
    }
    crate::admin::reload_and_swap(state)
}

/// 启动时调用：standalone 无 secret 模式 warn 一次。
pub fn warn_no_secret_once() {
    let secret = std::env::var("SEBAS_CONTROL_SECRET").unwrap_or_default();
    if secret.is_empty() {
        tracing::warn!(
            "[router] SEBAS_CONTROL_SECRET not set: admin surface accepts loopback only"
        );
    }
}

// -------------------- overlay 只读底座（外部热重载降级路径） --------------------/// providers.json 路径（与 config.rs 的 `provider_overlay` 同源；此处从
/// 当前内核 cfg 取——env 覆盖已在 parse 时应用）。
fn overlay_path(state: &AppState) -> PathBuf {
    PathBuf::from(state.core().cfg.provider_overlay.clone())
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

/// 从 config.toml 种子重建 RouterConfig，保留外壳启动期字段。
/// 不含 overlay 合并（调用方决定数据源：文件 or core channel 快照）。
pub(crate) fn rebuild_from_seed(state: &AppState) -> Result<RouterConfig, String> {
    let core = state.core();
    let toml_path = &core.cfg.config_source;
    let raw_toml = std::fs::read_to_string(toml_path)
        .map_err(|e| format!("读 config.toml ({toml_path}) 失败: {e}"))?;
    let mut cfg = RouterConfig::parse(&raw_toml).map_err(|e| format!("解析失败: {e}"))?;
    // 保留外壳的启动期字段（listen/超时等不因 reload 变化——它们来自原
    // cfg 而非新读）。
    cfg.listen = core.cfg.listen.clone();
    cfg.max_body_bytes = core.cfg.max_body_bytes;
    cfg.connect_timeout_secs = core.cfg.connect_timeout_secs;
    cfg.read_timeout_secs = core.cfg.read_timeout_secs;
    cfg.usage_file = core.cfg.usage_file.clone();
    cfg.debug = core.cfg.debug;
    cfg.rate_limit = core.cfg.rate_limit;
    // debug 模式的内置 test provider 是启动期内存注入、不在 config.toml
    // 里——rebuild 会丢。debug 标志保留时按幂等语义重注入，否则首次
    // admin 写（热替换）后 test 模型就 502 no_route。
    if cfg.debug {
        crate::debug::enable_debug_test_provider(&mut cfg);
    }
    Ok(cfg)
}

// -------------------- presets（只读代码表）--------------------

/// GET /admin/presets：内置 preset 表只读视图（直出运行中二进制的代码表，
/// 绝非存储副本——preset 数据跟随代码）。无 mutation 端点。
async fn list_presets() -> Response {
    let out: Vec<Value> = config::presets()
        .iter()
        .map(|p| {
            json!({
                "name": p.name,
                "base_url_anthropic": p.base_url_anthropic,
                "base_url_openai_chat": p.base_url_openai_chat,
                "base_url_openai_responses": p.base_url_openai_responses,
                "api_key_env": p.api_key_env,
                // 模型条目（id + 能力标记；wire 形状由 ModelEntry::Serialize
                // 统一承载，redesign-provider-models-settings task 1.2）。
                "models": p.models.iter().map(config::PresetModel::to_entry).collect::<Vec<_>>(),
            })
        })
        .collect();
    Json(json!({ "presets": out })).into_response()
}

// -------------------- reload / stats / metrics --------------------

/// POST /admin/reload：手动重读 + 热替换。成功返回摘要，失败返回错误文本。
async fn reload(State(state): State<AppState>) -> Response {
    // router-admin-api spec：/admin/reload 经状态库（channel state methods）按需
    // 重取配置——与写后 reload 同一条 channel-aware 管线（通道可用走快照投影，
    // 不可用回退文件 overlay）。
    match reload_after_write(&state).await {
        Ok(()) => Json(json!({"reloaded": true})).into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({"reloaded": false, "error": e})))
            .into_response(),
    }
}

/// GET /admin/stats：JSON 摘要供 webui 数字卡片渲染（router-metrics spec「JSON
/// stats summary」：uptime、全局 totals（requests / input|output|cache tokens /
/// rate-limited / upstream-errors）、per-provider 聚合（含平均延迟 ms）、末次
/// reload 状态）。registry 是进程级快照，包含全部历史 provider——已删除
/// provider 的残留计数保留（观测量，不因删除回零）。
async fn stats(State(state): State<AppState>) -> Response {
    let core = state.core();
    let m = crate::metrics::Metrics::global();
    let mut per_provider: BTreeMap<String, Value> = BTreeMap::new();
    // per-provider 平均延迟的临时累加器：sum/count（秒）。
    let mut lat_sum: BTreeMap<String, f64> = BTreeMap::new();
    let mut lat_count: BTreeMap<String, f64> = BTreeMap::new();
    // 全局 totals。
    let mut total_requests = 0.0f64;
    let mut total_rate_limited = 0.0f64;
    let mut total_upstream_errors = 0.0f64;
    let mut total_input = 0.0f64;
    let mut total_output = 0.0f64;
    let mut total_cache = 0.0f64;

    for (name, v) in m.snapshot() {
        let Some((labels, value)) = parse_series(&name, v) else {
            continue;
        };
        let val = value.as_f64().unwrap_or(0.0);
        if name.starts_with("router_requests_total") {
            total_requests += val;
        } else if name.starts_with("router_rate_limited_total") {
            total_rate_limited += val;
        } else if name.starts_with("router_upstream_errors_total") {
            total_upstream_errors += val;
        } else if name.starts_with("router_tokens_total") {
            match labels.get("type").map(String::as_str) {
                Some("input") => total_input += val,
                Some("output") => total_output += val,
                Some("cache_read") | Some("cache_creation") => total_cache += val,
                _ => {}
            }
        }
        if let Some(p) = labels.get("provider") {
            let p = p.clone();
            let e = per_provider.entry(p.clone()).or_insert_with(|| json!({"name": p}));
            if name.starts_with("router_requests_total") {
                e["requests"] = json!((e["requests"].as_f64().unwrap_or(0.0) + val) as u64);
            } else if name.starts_with("router_upstream_errors_total") {
                e["errors"] = json!((e["errors"].as_f64().unwrap_or(0.0) + val) as u64);
            } else if name.starts_with("router_tokens_total") {
                match labels.get("type").map(String::as_str) {
                    Some("input") => e["input_tokens"] = json!((e["input_tokens"].as_f64().unwrap_or(0.0) + val) as u64),
                    Some("output") => e["output_tokens"] = json!((e["output_tokens"].as_f64().unwrap_or(0.0) + val) as u64),
                    _ => {}
                }
            } else if name.starts_with("router_request_duration_seconds_sum") {
                *lat_sum.entry(p).or_insert(0.0) += val;
            } else if name.starts_with("router_request_duration_seconds_count") {
                *lat_count.entry(p).or_insert(0.0) += val;
            }
        }
    }
    // per-provider 平均延迟（ms）= sum(s) / count * 1000。
    for (p, entry) in per_provider.iter_mut() {
        let (s, c) = (lat_sum.get(p).copied().unwrap_or(0.0), lat_count.get(p).copied().unwrap_or(0.0));
        if c > 0.0 {
            entry["avg_latency_ms"] = json!(((s / c) * 1000.0 * 100.0).round() / 100.0);
        }
    }
    let mut out = json!({
        "uptime_secs": m.uptime_secs(),
        "providers": core.cfg.providers.len(),
        "routes": core.cfg.routes.len(),
        "totals": {
            "requests": total_requests as u64,
            "input_tokens": total_input as u64,
            "output_tokens": total_output as u64,
            "cache_tokens": total_cache as u64,
            "rate_limited": total_rate_limited as u64,
            "upstream_errors": total_upstream_errors as u64,
        },
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

/// 解析 series 名 → (labels, value)。非 sebas_router_* 前缀返回 None。
fn parse_series(name: &str, v: f64) -> Option<(BTreeMap<String, String>, Value)> {
    if !name.starts_with("router_") {
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
        let mtype = if base.contains("_duration_seconds_bucket") {
            // Prometheus 直方图：bucket 系列归并到 histogram 基名
            "histogram"
        } else if base.contains("_active_requests") || base.contains("_start_time_seconds") {
            "gauge"
        } else {
            "counter"
        };
        // 直方图基名（bucket/sum/count 共享一个 family 名）
        let family = if let Some(stripped) = base.strip_suffix("_bucket") {
            stripped
        } else if let Some(stripped) = base.strip_suffix("_sum") {
            stripped
        } else if let Some(stripped) = base.strip_suffix("_count") {
            stripped
        } else {
            base
        };
        if out.contains(&format!("# TYPE {family}")) {
            continue;
        }
        out.push_str(&format!("# TYPE {family} {mtype}\n"));
    }
    for (name, v) in m.snapshot() {
        out.push_str(&format!("{name} {v}\n"));
    }
    // router_start_time_seconds：进程启动时刻（gauge，unix 秒）。
    out.push_str("# TYPE router_start_time_seconds gauge\n");
    out.push_str(&format!(
        "router_start_time_seconds {}\n",
        m.start_time_unix()
    ));
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        out,
    )
        .into_response()
}
