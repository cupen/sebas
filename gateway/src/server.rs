//! axum server skeleton for the gateway (Task 2).
//!
//! `build_state` resolves upstream api keys, constructs the shared
//! `reqwest::Client` (connect/read timeouts), and builds the `RouteTable`.
//! `build_router` mounts the liveness probe `GET /healthz` (no auth,
//! `"ok\n"`) and the `proxy::handle` fallback for everything else, both
//! behind the `require_key` auth layer.
//!
//! `run` binds `cfg.listen` and serves with graceful shutdown (ctrl_c +
//! unix SIGTERM).

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::Router;
use axum::middleware::from_fn_with_state;
use axum::routing::get;

use crate::auth::require_key;
use crate::config::GatewayConfig;
use crate::error::{GatewayError, Result};
use crate::proxy;
use crate::rate_limit::rate_limit;
use crate::routing::RouteTable;
use crate::usage::UsageSink;

/// 可热替换的路由内核（design D1）：providers / api_keys / auth_tokens /
/// table 收拢于此，整体换入换出。`cfg` 是完整配置快照——外壳的 `cfg` 才是
/// 启动期字段（listen/超时等）的真源，内核里的这份只供路由路径取
/// providers/debug 等。全部字段 `Arc`/值语义，clone 纳秒级。
#[derive(Clone)]
pub struct GatewayCore {
    pub cfg: Arc<GatewayConfig>,
    /// 合法下游 token 集合（`[gateway] auth_token`，单串或数组）。
    pub auth_tokens: Arc<HashSet<String>>,
    pub api_keys: Arc<HashMap<String, String>>,
    pub table: Arc<RouteTable>,
}

impl GatewayCore {
    /// 从候选配置构建内核（resolve api keys + route table）。任何失败返回
    /// Err——`swap_core` 据此保证「校验失败不动旧内核」。
    pub fn build(cfg: GatewayConfig) -> Result<GatewayCore> {
        let api_keys = cfg.resolve_api_keys()?;
        let auth_tokens: HashSet<String> = cfg.auth_token.iter().cloned().collect();
        let table = RouteTable::from_config(&cfg);
        Ok(GatewayCore {
            cfg: Arc::new(cfg),
            auth_tokens: Arc::new(auth_tokens),
            api_keys: Arc::new(api_keys),
            table: Arc::new(table),
        })
    }
}

/// Shared server state. 外壳字段（client/sink/rate_limiter）启动期固定；
/// 路由相关字段经 `core`（`Arc<RwLock<GatewayCore>>`）可热替换。
/// 每请求一次 `read()` 快照（clone Arc 引用，纳秒级临界区，用 std RwLock）。
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<GatewayConfig>,
    pub core: Arc<RwLock<GatewayCore>>,
    pub client: reqwest::Client,
    pub sink: UsageSink,
    /// token-bucket 限流状态（`RateLimiter`：`Arc<Mutex>` → `Clone + Send + Sync`）。
    pub rate_limiter: crate::rate_limit::RateLimiter,
    /// 热重载状态（last_reload_error / last_ok_at，供 /admin/stats）。
    pub reload_status: Arc<crate::hot_reload::ReloadStatus>,
}

impl AppState {
    /// 读锁取内核快照。锁中毒 = 内核写方 panic，此时保旧继续服务（unwrap_or_else
    /// poison 恢复：RwLock 数据实际完好）。
    pub fn core(&self) -> GatewayCore {
        self.core
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 热替换内核（design D2/3.5/4.x）：先完整构建候选（校验失败 → Err 不动
    /// 旧内核），写锁整体替换。在途请求继续持有旧快照，天然免中断。
    pub fn swap_core(&self, candidate: GatewayConfig) -> Result<()> {
        let new_core = GatewayCore::build(candidate)?;
        let mut guard = self
            .core
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = new_core;
        Ok(())
    }
}

/// Resolve api keys + build the upstream client + route table. Called once
/// at startup.
pub fn build_state(cfg: GatewayConfig) -> Result<AppState> {
    let auth_tokens: HashSet<String> = cfg.auth_token.iter().cloned().collect();
    if auth_tokens.is_empty() {
        tracing::warn!(
            "[gateway] 未配置 auth_token：不校验下游 token（裸奔）。生产环境请配置 auth_token。"
        );
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(cfg.connect_timeout_secs))
        // per-read timeout: resets on activity, so long SSE streams that keep
        // emitting are not cut. This is the "read timeout" the spec calls out
        // (§4.3, 放宽到分钟级) — distinct from a hard total `.timeout()`.
        .read_timeout(Duration::from_secs(cfg.read_timeout_secs))
        .build()
        .map_err(|e| GatewayError::Upstream(format!("构建 reqwest client 失败: {e}")))?;
    // usage jsonl sink：spawn_writer 起后台 task + 建父目录。失败 → Config
    // 错误拒绝启动（spec：usage_file 父目录由 spawn_writer 创建）。
    let sink = UsageSink::spawn_writer(&cfg.usage_file).map_err(|e| {
        GatewayError::Config(format!(
            "usage sink (path {}) spawn failed: {e}",
            cfg.usage_file
        ))
    })?;
    // token-bucket 限流状态（`cfg.rate_limit` 缺省不限流）。
    let rate_limiter = crate::rate_limit::RateLimiter::from_config(&cfg.rate_limit);
    let core = GatewayCore::build(cfg)?;
    let reload_status = crate::hot_reload::ReloadStatus::new();
    Ok(AppState {
        cfg: core.cfg.clone(),
        core: Arc::new(RwLock::new(core)),
        client,
        sink,
        rate_limiter,
        reload_status,
    })
}

/// Mount routes. `GET /healthz` + `proxy::handle` fallback, both behind the
/// `require_key` auth layer. The layer sits above the fallback so `/healthz`
/// also passes through `require_key`, which exempts it by path. Everything
/// else flows through `proxy::handle`.
pub fn build_router(state: AppState) -> Router {
    // admin 面（/admin/* + /metrics）：独立鉴权（admin_auth 在
    // build_admin_router 内部），不受 require_key/rate_limit 影响。
    // 先把主 router（proxy fallback）收敛为 () state，再 merge 同为 ()
    // 的 admin router——admin 路由因此挂在 fallback 之上，不被 proxy 吞。
    let admin = crate::admin::build_admin_router(state.clone());
    Router::new()
        .route("/healthz", get(healthz))
        .fallback(proxy::handle)
        // 内层：token-bucket 限流。挂在鉴权之内，只对放行的合法请求计数
        // （按鉴权后的 client/token 维度，见 rate_limit.rs）。
        .layer(from_fn_with_state(state.clone(), rate_limit))
        // 鉴权层：在内层之上，先鉴权再限流。
        .layer(from_fn_with_state(state.clone(), require_key))
        // 最外层：nginx 风格 access log，覆盖 /healthz 与全部透传请求。
        .layer(axum::middleware::from_fn(crate::access_log::access_log))
        .with_state(state)
        .merge(admin)
}

/// Liveness probe — no auth, no state, returns literal `"ok\n"`.
async fn healthz() -> &'static str {
    "ok\n"
}

/// Bind `cfg.listen`, serve with graceful shutdown (ctrl_c + unix SIGTERM).
pub async fn run(cfg: GatewayConfig) -> Result<()> {
    let listen = cfg.listen.clone();
    let state = build_state(cfg)?;
    crate::admin::warn_no_secret_once();
    crate::hot_reload::spawn_watcher(state.clone(), state.reload_status.clone());
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, "sebas gateway listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

/// 以外部提供的 listener 启动 gateway（不接管 ctrl_c/SIGTERM），返回实际监听
/// 地址与 serve task。供嵌入方（`sebas run --gateway`）在随机端口（`127.0.0.1:0`）
/// 上启动并向日志输出真实端口；进程生命周期由调用方/runtime 管理。
pub fn serve_with_listener(
    cfg: GatewayConfig,
    listener: tokio::net::TcpListener,
) -> Result<(
    std::net::SocketAddr,
    tokio::task::JoinHandle<std::io::Result<()>>,
)> {
    let state = build_state(cfg)?;
    crate::hot_reload::spawn_watcher(state.clone(), state.reload_status.clone());
    let app = build_router(state);
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
    });
    Ok((addr, handle))
}

/// Wait for ctrl_c (any platform) or SIGTERM (unix). First signal wins.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install ctrl_c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("gateway shutdown signal received, draining connections");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::WireProtocol;

    fn test_cfg(provider_names: &[&str]) -> GatewayConfig {
        let mut providers = HashMap::new();
        for n in provider_names {
            providers.insert(
                (*n).to_string(),
                crate::config::ProviderConfig {
                    base_url_anthropic: Some(format!("https://{n}.example")),
                    base_url_openai: None,
                    api_key: Some("test-key".into()),
                    api_key_env: None,
                    model_map: HashMap::new(),
                    models: Vec::new(),
                },
            );
        }
        GatewayConfig {
            listen: "127.0.0.1:0".into(),
            max_body_bytes: 1024,
            connect_timeout_secs: 1,
            read_timeout_secs: 1,
            usage_file: "/tmp/sebas-gw-test-usage.jsonl".into(),
            debug: false,
            provider_overlay: "__test_no_overlay__.json".into(),
            default_provider: None,
            auth_token: Vec::new(),
            rate_limit: crate::config::RateLimitConfig::default(),
            providers,
            routes: Vec::new(),
            model_aliases: HashMap::new(),
            config_source: "/__test_no_config__.toml".into(),
        }
    }

    /// swap 成功后新请求用新 table；swap 失败（缺 URL 的 provider）旧内核保持。
    #[tokio::test]
    async fn swap_core_replaces_and_keeps_old_on_failure() {
        let state = build_state(test_cfg(&["alpha", "beta"])).expect("build_state");
        // 初始：唯一隐式默认不存在（两个 provider），无 routes —— alpha 不可
        // 直接解析。先 swap 成只有 alpha 的候选。
        let candidate = test_cfg(&["alpha"]);
        state.swap_core(candidate).expect("swap ok");
        let core = state.core();
        let d = core
            .table
            .resolve(Some("anything"), WireProtocol::Anthropic)
            .expect("唯一 provider 隐式默认");
        assert_eq!(d.provider, "alpha");

        // 失败候选（provider 无任何 URL）：swap 报错，旧内核保持 alpha。
        let mut bad = test_cfg(&[]);
        bad.providers.insert(
            "gamma".into(),
            crate::config::ProviderConfig {
                base_url_anthropic: None,
                base_url_openai: None,
                api_key: None,
                api_key_env: None,
                model_map: HashMap::new(),
                models: Vec::new(),
            },
        );
        let err = state.swap_core(bad).expect_err("无效候选须报错");
        assert!(err.to_string().contains("gamma"), "错误含 provider 名: {err}");
        let core = state.core();
        let d = core
            .table
            .resolve(Some("anything"), WireProtocol::Anthropic)
            .expect("旧内核继续服务");
        assert_eq!(d.provider, "alpha", "swap 失败不动旧内核");
    }
}
