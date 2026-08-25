//! 主运行编排：装配各子系统（router/manager/feishu/ws）并跑到信号退出。
//!
//! 职责拆分（原 run.rs 922 行）：
//! - 出站分发 `Out` → 副作用： [`crate::dispatch`]
//! - ACP 会话生命周期（spawn/resume/pump）: [`crate::session_boot`]
//! - 飞书 WS 事件循环： [`crate::ws_loop`]
//!
//! 下面的 re-export 是 facade：integration tests 与 `replay` 仍走
//! `sebas::run::{...}` 路径，拆模块不牵动调用方。

pub use crate::session_boot::{
    acp_resume_and_activate, acp_spawn_and_activate, flush_pending_prompts, restore_session_map,
    spawn_acp_pump,
};
pub use crate::ws_loop::RouterEventHandler;

use crate::config::Config;
use crate::dispatch::dispatch_out;
use crate::error::Result;
use crate::reactions::ReactionTracker;
use crate::ws_loop::{run_ws_loop, spawn_test_session};
use acp_claude::manager::SessionManager;
use feishu::client::{FeishuClient, FeishuConfig};
use feishu::messages::{ReceiveIdType, SendTextRequest};
use gateway::config::GatewayConfig;
use router::router::RouterHandle;
use router::settings;
use std::sync::Arc;
use tracing::{error, info, warn};

pub async fn run(
    cfg: Config,
    raw_config: String,
    test_msg: Option<String>,
    dump_inbound: Option<String>,
    gateway_cfg: Option<GatewayConfig>,
    webui: bool,
    webui_port: u16,
) -> Result<()> {
    // 在 watchdog 下运行时初始化 IPC
    if crate::ipc::is_under_watchdog() {
        init_watchdog_ipc().await;
    }

    // openlark 0.19 uses reqwest 0.13, whose Rustls connector consults the
    // process-wide provider. Our reqwest 0.12 clients use ring explicitly;
    // install one provider up front so the mixed dependency graph is
    // deterministic instead of panicking when both providers are compiled.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    init_tracing(&cfg);

    // spec §6.4 startup checks: directories writable + ACP binary reachable.
    // Friendly Config error, no panic; runs before any network/spawn work.
    cfg.validate_runtime()?;

    // `run --gateway`：在随机端口上启动内置 gateway，实际端口记入日志
    // （调用方按需把 ANTHROPIC_BASE_URL/OPENAI_BASE_URL 指向该地址）。
    if let Some(ref gw_cfg) = gateway_cfg {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| crate::error::SebasError::Gateway(format!("绑定随机端口失败: {e}")))?;
        let (addr, _handle) = gateway::server::serve_with_listener(gw_cfg.clone(), listener)
            .map_err(|e| crate::error::SebasError::Gateway(e.to_string()))?;
        info!(%addr, "gateway started (run --gateway); point ANTHROPIC_BASE_URL/OPENAI_BASE_URL at {}", format!("http://{addr}"));
    }

    if cfg.feishu.owner_id.is_empty() {
        // owner_id 决策（sebas-nya，文档化于 config.rs validate）：可选。
        // 空值 = 不过滤发送者 —— 对能执行任意命令的单用户 bot 是真实风险，
        // 启动时必须醒目提示。
        warn!(
            "feishu.owner_id 为空：任何飞书用户的消息都会被处理并驱动本机 claude；\
             单用户机器人建议配置 owner_id（spec §6.1）"
        );
    }

    if !crate::ipc::is_under_watchdog()
        && std::env::var_os("SEBAS_CONTROL_SECRET")
            .map(|v| v.is_empty())
            .unwrap_or(true)
    {
        // 裸 core 启动：/upgrade / /rollback / /gateway 等需要 watchdog RPC 的命令
        // 在此模式下不可用。启动时给可执行提示，避免用户调用 /upgrade 后才发现。
        warn!(
            "当前为裸 core 启动模式（SEBAS_IPC 未设置 + SEBAS_CONTROL_SECRET 未配置）：\
             /upgrade、/rollback、/restart、/gateway 等命令需要 watchdog 转发，\
             在此模式下调用会失败。如需启用，请通过 `sebas` watchdog 启动 core（spec §5.3）"
        );
    }

    let map = restore_session_map(&cfg.router.state_file, cfg.router.max_concurrent_sessions);

    // TOML is bootstrap; settings.json (if present) wins wholesale.
    // Strict: malformed settings.json refuses to start with a clear error.
    // Missing settings.json → fall back to TOML [card] so first-boot users
    // get the configured values rather than serde defaults.
    let merged_card_cfg = match settings::load_settings(&settings::settings_path()) {
        Ok(Some(s)) => s,
        Ok(None) => cfg.card.clone(),
        Err(e) => {
            error!(error = %e, "settings.json 解析失败，拒绝启动");
            return Err(crate::error::SebasError::Config(e));
        }
    };
    let mgr = Arc::new(SessionManager::new(std::time::Duration::from_secs(
        cfg.acp.claude.startup_timeout_secs,
    )));
    let provider_forms = crate::provider::build_form(&raw_config);
    let (router, mut out_rx) = RouterHandle::new_with_provider_form(
        map,
        merged_card_cfg,
        cfg.router.channel_buffer,
        provider_forms,
        Some(mgr.clone()),
    );
    // Tracks the current emoji reaction on each session's root card so the
    // router's phase machine can swap 👀→🚧→✅ rather than pile them up.
    let reactions = Arc::new(ReactionTracker::default());

    let feishu = FeishuClient::new(FeishuConfig {
        app_id: cfg.feishu.app_id.clone(),
        app_secret: cfg.feishu.app_secret.clone(),
        owner_id: cfg.feishu.owner_id.clone(),
    });

    let http = reqwest::Client::new();
    // Test affordance: `SEBAS_TEST_FAKE_TOKEN=1` skips the live Feishu auth
    // HTTP call and substitutes a stub token. Used by integration tests that
    // cannot reach the live Feishu API. Off by default; production callers
    // see no behaviour change.
    let tokens = if std::env::var("SEBAS_TEST_FAKE_TOKEN").as_deref() == Ok("1") {
        info!("SEBAS_TEST_FAKE_TOKEN=1; using stub tenant_access_token");
        feishu::client::TokenManager::with_stub_token("t-stub-test")
    } else {
        let tm = feishu::client::TokenManager::new(
            cfg.feishu.app_id.clone(),
            cfg.feishu.app_secret.clone(),
        );
        // Startup auth check stays fatal (spec §4.1).
        tm.token()
            .await
            .map_err(|e| crate::error::SebasError::Feishu(e.to_string()))?;
        tm
    };

    // hello_msg: send to the owner (private DM via open_id) if both are set.
    // If owner_id is empty, do nothing.
    if !cfg.feishu.hello_msg.is_empty() && !cfg.feishu.owner_id.is_empty() {
        let url = "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=open_id";
        let req = SendTextRequest::new(
            &cfg.feishu.owner_id,
            ReceiveIdType::OpenId,
            &cfg.feishu.hello_msg,
        );
        let body = serde_json::to_value(&req).unwrap_or_default();
        let bearer = tokens.token().await.unwrap_or_default();
        match http.post(url).bearer_auth(&bearer).json(&body).send().await {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                info!(%status, body = %body, "hello_msg send result");
            }
            Err(e) => warn!(?e, "hello_msg send failed"),
        }
    }

    // Optional startup test message: send "sebas 已启动" to the given receive_id
    // (interpreted as chat_id; for private DMs to a user, pass their open_id and
    // set receive_id_type=open_id below). Default to chat_id for groups.
    if let Some(receive_id) = test_msg {
        let url = "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id";
        let req = SendTextRequest::new(receive_id, ReceiveIdType::ChatId, "✅ sebas 已启动");
        let body = serde_json::to_value(&req).unwrap_or_default();
        async {
            let bearer = tokens.token().await.unwrap_or_default();
            let resp = http
                .post(url)
                .bearer_auth(&bearer)
                .json(&body)
                .send()
                .await
                .map_err(|e| crate::error::SebasError::Feishu(format!("send: {e}")))?;
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            info!(%status, body = %body, "test message send result");
            if !status.is_success() {
                Err(crate::error::SebasError::Feishu(format!(
                    "test message failed: {body}"
                )))
            } else {
                Ok(())
            }
        }
        .await?;
    }

    // Spawn outbound pump
    let cfg_for_outbound = cfg.clone();
    let tokens_for_outbound = tokens.clone();
    let http_for_outbound = http.clone();
    let feishu_for_outbound = feishu.clone();
    let router_for_outbound = router.clone();
    let mgr_for_outbound = mgr.clone();
    let reactions_for_outbound = reactions.clone();
    let gateway_cfg_for_outbound = gateway_cfg.clone();
    tokio::spawn(async move {
        while let Some(out) = out_rx.recv().await {
            if let Err(e) = dispatch_out(
                &feishu_for_outbound,
                &http_for_outbound,
                &tokens_for_outbound,
                &cfg_for_outbound,
                &router_for_outbound,
                &mgr_for_outbound,
                &reactions_for_outbound,
                gateway_cfg_for_outbound.as_ref(),
                out,
            )
            .await
            {
                error!(?e, "outbound dispatch failed");
            }
        }
    });

    // Start WebUI dashboard server if requested
    if webui {
        let router_for_webui = router.clone();
        let mgr_for_webui = mgr.clone();
        let gateway_info = build_gateway_info(gateway_cfg.as_ref());
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{webui_port}"))
            .await
            .map_err(|e| crate::error::SebasError::Gateway(format!("绑定 webui 端口失败: {e}")))?;
        tokio::spawn(async move {
            webui::run(router_for_webui, mgr_for_webui, gateway_info, listener).await;
        });
        info!("webui dashboard starting on 127.0.0.1:{webui_port}");
    }

    // Run the long-connection event loop inline in a `tokio::select!` so the
    // shutdown signal can drop the WebSocket future and close the connection
    // promptly. If the reconnect loop ever exits, keep waiting for ctrl_c so
    // the normal session cleanup and state snapshot still run.
    let ws_router = router.clone();
    let ws_owner = cfg.feishu.owner_id.clone();
    let ws_app_id = cfg.feishu.app_id.clone();
    let ws_app_secret = cfg.feishu.app_secret.clone();
    let ws_dump_dir = match dump_inbound.as_ref() {
        Some(p) => match std::fs::create_dir_all(p) {
            Ok(()) => Some(std::path::PathBuf::from(p)),
            Err(e) => {
                warn!(?e, path = %p, "failed to create inbound dump dir; disabling dump");
                None
            }
        },
        None => None,
    };
    if let Some(d) = &ws_dump_dir {
        info!(dir = %d.display(), "inbound WS payloads will be dumped here");
    }

    // Test affordance: `SEBAS_TEST_SPAWN_SESSION=1` mints a session via the
    // `acp.claude.path` binary at startup. Without this, the daemon idles
    // and no child is ever spawned — which makes the SIGTERM-cleanup test
    // vacuous. With it, an ACP child is alive as a direct descendant of
    // the sebas pid, so `kill_all` actually has work to do. Off by default.
    if std::env::var("SEBAS_TEST_SPAWN_SESSION").as_deref() == Ok("1") {
        spawn_test_session(&cfg, &router, &mgr).await;
    }

    info!("sebas started; waiting for SIGINT/SIGTERM");
    let sigint = async {
        tokio::signal::ctrl_c().await.ok();
    };
    let sigterm = async {
        #[cfg(unix)]
        {
            let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
            sig.recv().await;
        }
        #[cfg(not(unix))]
        {
            // Non-unix platforms only have ctrl_c equivalent; never fires
            // separately here. Block forever so the select arm stays inert.
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        _ = sigint => {
            info!("shutting down (SIGINT)");
        }
        _ = sigterm => {
            info!("shutting down (SIGTERM)");
        }
        _ = run_ws_loop(&ws_app_id, &ws_app_secret, &ws_owner, ws_router, ws_dump_dir, cfg.feishu.allowed_chat_types.clone(), cfg.feishu.bot_name.clone()) => {
            warn!("WS loop exited; awaiting ctrl_c");
            tokio::signal::ctrl_c().await.ok();
        }
    }

    // Snapshot state BEFORE killing children (spec §4.2 order: dump, then
    // shutdown_children). Dumping after kill_all would race the pumps'
    // teardown (terminal events strip mappings) and would lose the whole
    // snapshot if a child hangs the kill — the restored mappings are what
    // lazy respawn (spec §3.3e) works from.
    let json = router
        .dump_json()
        .await
        .map_err(|e| crate::error::SebasError::Router(e.to_string()))?;
    if let Err(e) = std::fs::write(&cfg.router.state_file, json) {
        warn!(?e, "failed to persist session state");
    }

    // Signal all live sessions to cancel and reap their child processes.
    mgr.kill_all().await;
    Ok(())
}

fn init_tracing(cfg: &Config) {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_new(&cfg.log.level).unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = fmt().with_env_filter(filter);
    if let Some(ref path) = cfg.log.file
        && let Ok(file) = std::fs::File::create(path)
    {
        subscriber.with_writer(file).init();
        return;
    }
    // watchdog 下 stdout 是 IPC 管道（Ready-only 协议）：日志必须写 stderr
    // （父进程 inherit），否则读端在 ready 后排空时会与日志争用同一管道。
    if crate::ipc::is_under_watchdog() {
        subscriber.with_writer(std::io::stderr).init();
        return;
    }
    subscriber.init();
}

/// Build a GatewayInfo from the optional gateway config for the WebUI.
fn build_gateway_info(gateway_cfg: Option<&GatewayConfig>) -> webui::models::GatewayInfo {
    let Some(gw) = gateway_cfg else {
        return webui::models::GatewayInfo::default();
    };
    let providers = gw
        .providers
        .iter()
        .map(|(name, p)| webui::models::ProviderInfo {
            name: name.clone(),
            base_url_anthropic: p.base_url_anthropic.clone(),
            base_url_openai: p.base_url_openai.clone(),
        })
        .collect();
    webui::models::GatewayInfo {
        listen: Some(gw.listen.clone()),
        provider_count: gw.providers.len(),
        debug: gw.debug,
        has_auth: !gw.auth_token.is_empty(),
        providers,
    }
}

/// 在 watchdog 下运行时向父进程发送 ready 握手（Ready-only 协议）。
/// 控制命令一律走 control RPC（Unix socket），pipe 不再承载命令。
async fn init_watchdog_ipc() {
    use tracing::info;

    let mut ipc = crate::ipc::ChildIpc::new();
    if let Err(e) = ipc.ready().await {
        tracing::warn!("watchdog IPC ready 发送失败: {e}");
        return;
    }
    info!("watchdog IPC 连接就绪");
}
