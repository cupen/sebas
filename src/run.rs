use crate::config::Config;
use crate::error::Result;
use acp_claude::manager::SessionManager;
use acp_claude::session::{AcpCommand, AcpEvent};
use feishu::cards::render_root_card;
use feishu::client::{FeishuClient, FeishuConfig};
use open_lark::ws_client::{EventDispatcherHandler, EventHandler, LarkWsClient, WsClientError};
use open_lark::Config as LarkConfig;
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, debug, warn, error};

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

    let state_raw = std::fs::read_to_string(&cfg.router.state_file)
        .unwrap_or_else(|_| "{}".into());
    let map = SessionMap::restore_json(&state_raw)
        .map_err(|e| crate::error::SebasError::Config(format!("restore: {e}")))?;

    let (router, mut out_rx) = RouterHandle::new(map);
    let mgr = Arc::new(SessionManager::new());

    let feishu = FeishuClient::new(FeishuConfig {
        app_id: cfg.feishu.app_id.clone(),
        app_secret: cfg.feishu.app_secret.clone(),
        owner_id: cfg.feishu.owner_id.clone(),
    });

    let http = reqwest::Client::new();
    let token = feishu
        .fetch_token(&http)
        .await
        .map_err(|e| crate::error::SebasError::Feishu(e.to_string()))?;

    // hello_msg: send to the owner (private DM via open_id) if both are set.
    // If owner_id is empty, do nothing.
    if !cfg.feishu.hello_msg.is_empty() && !cfg.feishu.owner_id.is_empty() {
        let url = "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=open_id";
        let body = serde_json::json!({
            "receive_id": cfg.feishu.owner_id,
            "msg_type": "text",
            "content": serde_json::to_string(&serde_json::json!({"text": cfg.feishu.hello_msg})).unwrap_or_default(),
        });
        match http.post(url).bearer_auth(&token.access_token).json(&body).send().await {
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
        let result = async {
            let resp = http.post(url).bearer_auth(&token.access_token).json(&body).send().await
                .map_err(|e| crate::error::SebasError::Feishu(format!("send: {e}")))?;
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            info!(%status, body = %body, "test message send result");
            if !status.is_success() {
                Err(crate::error::SebasError::Feishu(format!("test message failed: {body}")))
            } else {
                Ok(())
            }
        }.await;
        if let Err(e) = result {
            return Err(e);
        }
    }

    // Spawn outbound pump
    let cfg_for_outbound = cfg.clone();
    let token_clone = token.access_token.clone();
    let http_for_outbound = http.clone();
    let feishu_for_outbound = feishu.clone();
    let router_for_outbound = router.clone();
    let mgr_for_outbound = mgr.clone();
    tokio::spawn(async move {
        while let Some(out) = out_rx.recv().await {
            if let Err(e) = dispatch_out(
                &feishu_for_outbound,
                &http_for_outbound,
                &token_clone,
                &cfg_for_outbound,
                &router_for_outbound,
                &mgr_for_outbound,
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

    info!("sebas started; waiting for SIGINT/SIGTERM");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("shutting down (SIGINT)");
        }
        _ = run_ws_loop(&ws_app_id, &ws_app_secret, &ws_owner, ws_router, ws_dump_dir) => {
            warn!("WS loop exited; awaiting ctrl_c");
            tokio::signal::ctrl_c().await.ok();
        }
    }

    // Signal all live sessions to cancel and reap their child processes before
    // snapshotting state.
    mgr.kill_all().await;

    // Dump sessions on exit
    let json = router
        .dump_json()
        .await
        .map_err(|e| crate::error::SebasError::Router(e.to_string()))?;
    if let Err(e) = std::fs::write(&cfg.router.state_file, json) {
        warn!(?e, "failed to persist session state");
    }
    Ok(())
}

async fn dispatch_out(
    feishu: &FeishuClient,
    http: &reqwest::Client,
    token: &str,
    cfg: &Config,
    router: &RouterHandle,
    mgr: &Arc<SessionManager>,
    out: Out,
) -> anyhow::Result<()> {
    match out {
        Out::SendCard { key, card, msg_id } => {
            // The MsgIdMap is keyed by session_id (never chat_id) so that
            // `UpdateCard`/`React`, which only know the session_id, can resolve
            // the message_id. Only record when a session_id is supplied; plain
            // cards (permission prompts, help) don't need to be updated later.
            let new_id = feishu.send_card(http, token, &key, card).await?;
            if let (false, Some(session_id)) = (new_id.is_empty(), msg_id) {
                router.record_root_msg_id(session_id, new_id.clone()).await;
                debug!(message_id = %new_id, "recorded card msg_id");
            }
        }
        Out::UpdateCard { session_id, card } => {
            if let Some(message_id) = router.root_msg_id(&session_id).await {
                feishu.update_card(http, token, &message_id, card).await?;
            } else {
                debug!(?session_id, "no root msg_id recorded; skipping update");
            }
        }
        Out::React { session_id, emoji } => {
            if let Some(message_id) = router.root_msg_id(&session_id).await {
                feishu.react(http, token, &message_id, &emoji).await?;
            } else {
                debug!(?session_id, "no root msg_id recorded; skipping react");
            }
        }
        Out::SpawnAcp { key, prompt } => {
            let claude = &cfg.acp.claude;
            // 1) Spawn the claude subprocess and mint a session_id.
            let session_id = mgr
                .create_session(
                    &claude.path,
                    claude.args.clone(),
                    claude.work_dir.clone(),
                    prompt.clone(),
                )
                .await?;
            // 2) Kick off the session with the initial prompt.
            mgr.send(
                &session_id,
                AcpCommand::CreateSession {
                    session_id: session_id.clone(),
                    prompt: prompt.clone(),
                },
            )
            .await?;
            // 3) Record the mapping so continuations, permission-card routing
            //    and liveness checks can find this session.
            router.insert_mapping(key.clone(), session_id.clone()).await;
            // 4) Send the root card and record its message_id keyed by the real
            //    session_id (so streaming UpdateCards resolve correctly). Done
            //    before the event pump starts so no early delta is lost.
            let card = render_root_card(&prompt, &session_id, "👀");
            let msg_id = feishu
                .send_card(http, token, &key, serde_json::to_value(&card)?)
                .await?;
            if !msg_id.is_empty() {
                router.record_root_msg_id(session_id.clone(), msg_id).await;
            }
            // 5) Pump ACP events from this session back into the router.
            spawn_acp_pump(mgr.clone(), router.clone(), session_id);
        }
        Out::SendAcp { session_id, cmd } => {
            mgr.send(&session_id, cmd).await?;
        }
        Out::HelpText { key } => {
            info!(?key, "send help");
        }
    }
    Ok(())
}

/// Drain ACP events for one session and forward each into the router, which
/// turns them into `UpdateCard` / `SendCard` outbound messages. Exits when the
/// session's event stream closes (process exited / stdout EOF).
fn spawn_acp_pump(mgr: Arc<SessionManager>, router: RouterHandle, session_id: String) {
    tokio::spawn(async move {
        let Some(rx) = mgr.event_rx(&session_id).await else {
            warn!(%session_id, "no event_rx for session; pump not started");
            return;
        };
        let mut rx = rx.lock().await;
        while let Some(evt) = rx.recv().await {
            let finished = matches!(evt, AcpEvent::Finished { .. } | AcpEvent::Error { .. });
            router.dispatch_acp_event(evt).await;
            if finished {
                debug!(%session_id, "session reported completion");
            }
        }
        debug!(%session_id, "acp event stream closed; pump exiting");
    });
}

/// Long-connection WebSocket loop driven by `open-lark`. The crate handles the
/// protobuf framing and the `/callback/ws/endpoint` handshake for us, so all
/// we have to do is register a raw event handler on the dispatcher and
/// forward each inbound message into the router.
///
/// `LarkWsClient::open` returns when the server closes the connection (or on
/// any other error); we wrap it in an outer reconnect loop with exponential
/// backoff so a transient flap doesn't take the bot offline.
///
/// Note on event coverage: we register only `im.message.receive_v1` via
/// `register_raw` (v0.19.0+). That covers inbound text/media. Other event
/// types such as `card.action.trigger` (used by our permission card
/// buttons) are not surfaced by this dispatcher; see the migration report at
/// `.superpowers/sdd/ws-v019-cratesio-report.md` for the alternatives.
async fn run_ws_loop(
    app_id: &str,
    app_secret: &str,
    owner_id: &str,
    router: RouterHandle,
    dump_dir: Option<std::path::PathBuf>,
) {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(60);

    loop {
        // Rebuild the dispatcher for each connection attempt so retries start
        // with a fresh handler and cheap clones of the router and owner ID.
        let handler = RouterEventHandler {
            router: router.clone(),
            owner_id: owner_id.to_string(),
            dump_dir: dump_dir.clone(),
        };
        let dispatcher = match EventDispatcherHandler::builder()
            .register_raw("im.message.receive_v1", handler)
        {
            Ok(builder) => builder.build(),
            Err(e) => {
                error!(error = %e, "failed to register event handlers; aborting WS loop");
                return;
            }
        };

        let ws_config = LarkConfig::builder()
            .app_id(app_id.to_string())
            .app_secret(app_secret.to_string())
            .build();
        let ws_config = Arc::new(ws_config);

        info!("connecting to feishu WS via open-lark");
        let result = LarkWsClient::open(ws_config, dispatcher).await;

        match result {
            Ok(()) => {
                info!("feishu WS session ended cleanly; reconnecting");
                backoff = Duration::from_secs(1);
            }
            Err(WsClientError::ConnectionClosed { reason }) => {
                warn!(?reason, "feishu WS closed; reconnecting");
                backoff = Duration::from_secs(1);
            }
            Err(e) => {
                warn!(error = %e, "feishu WS failed; backing off");
            }
        }

        info!(?backoff, "WS reconnect after backoff");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

/// Raw-bytes event handler bound to `im.message.receive_v1` via
/// `register_raw`. Bypasses v0.14.0's typed-registration bug (where the
/// dispatcher built the lookup key as `schema.type_` instead of the
/// server-emitted `p2.*` key, dropping every inbound message) by avoiding
/// the typed dispatch layer entirely: we get the framed JSON payload, parse
/// it as our own `FeishuEnvelope`, and forward into the router.
struct RouterEventHandler {
    router: RouterHandle,
    owner_id: String,
    /// Optional directory for raw payload snapshots. When set, every received
    /// WS frame is written to `<dir>/<unix_ms>-<uuid>.json` before parsing, so
    /// you can replay captured traffic locally without a live Feishu bot.
    dump_dir: Option<std::path::PathBuf>,
}

impl EventHandler for RouterEventHandler {
    fn handle(
        &self,
        payload: &[u8],
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(dir) = &self.dump_dir {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let pid = std::process::id();
            let path = dir.join(format!("{ts}-{pid}.json"));
            if let Err(e) = std::fs::write(&path, payload) {
                warn!(?e, ?path, "failed to dump inbound payload");
            }
        }
        let text = std::str::from_utf8(payload)?;
        match serde_json::from_str::<feishu::events::FeishuEnvelope>(text) {
            Ok(env) => {
                if let Some(in_ev) = env.into_event(&self.owner_id) {
                    let router = self.router.clone();
                    // dispatcher handler is sync; dispatch is async → spawn
                    tokio::spawn(async move {
                        router.dispatch(in_ev).await;
                    });
                }
            }
            Err(e) => warn!(?e, "failed to parse open-lark envelope"),
        }
        Ok(())
    }
}

fn init_tracing(cfg: &Config) {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_new(&cfg.log.level).unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = fmt().with_env_filter(filter);
    if let Some(ref path) = cfg.log.file {
        if let Ok(file) = std::fs::File::create(path) {
            subscriber.with_writer(file).init();
            return;
        }
    }
    subscriber.init();
}