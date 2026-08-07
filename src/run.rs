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
use router::router::RouterHandle;
use std::sync::Arc;
use tracing::{error, info, warn};

pub async fn run(
    cfg: Config,
    test_msg: Option<String>,
    dump_inbound: Option<String>,
) -> Result<()> {
    // openlark 0.19 uses reqwest 0.13, whose Rustls connector consults the
    // process-wide provider. Our reqwest 0.12 clients use ring explicitly;
    // install one provider up front so the mixed dependency graph is
    // deterministic instead of panicking when both providers are compiled.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    init_tracing(&cfg);

    // spec §6.4 startup checks: directories writable + ACP binary reachable.
    // Friendly Config error, no panic; runs before any network/spawn work.
    cfg.validate_runtime()?;

    if cfg.feishu.owner_id.is_empty() {
        // owner_id 决策（sebas-nya，文档化于 config.rs validate）：可选。
        // 空值 = 不过滤发送者 —— 对能执行任意命令的单用户 bot 是真实风险，
        // 启动时必须醒目提示。
        warn!(
            "feishu.owner_id 为空：任何飞书用户的消息都会被处理并驱动本机 claude；\
             单用户机器人建议配置 owner_id（spec §6.1）"
        );
    }

    let map = restore_session_map(&cfg.router.state_file, cfg.router.max_concurrent_sessions);

    let (router, mut out_rx) =
        RouterHandle::new_with_config(map, cfg.card.clone(), cfg.router.channel_buffer);
    let mgr = Arc::new(SessionManager::new(std::time::Duration::from_secs(
        cfg.acp.claude.startup_timeout_secs,
    )));
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
        let body = serde_json::json!({
            "receive_id": cfg.feishu.owner_id,
            "msg_type": "text",
            "content": serde_json::to_string(&serde_json::json!({"text": cfg.feishu.hello_msg})).unwrap_or_default(),
        });
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
        let body = serde_json::json!({
            "receive_id": receive_id,
            "msg_type": "text",
            "content": serde_json::to_string(&serde_json::json!({"text": "✅ sebas 已启动"})).unwrap_or_default(),
        });
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
                out,
            )
            .await
            {
                error!(?e, "outbound dispatch failed");
            }
        }
    });

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
        _ = run_ws_loop(&ws_app_id, &ws_app_secret, &ws_owner, ws_router, ws_dump_dir) => {
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
    subscriber.init();
}
