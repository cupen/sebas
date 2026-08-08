use crate::config::Config;
use crate::error::Result;
use crate::reactions::{ReactPlan, ReactionTracker};
use acp_claude::manager::SessionManager;
use acp_claude::session::{AcpCommand, AcpEvent};
use feishu::cards::render_accumulated_card;
use feishu::client::{FeishuClient, FeishuConfig};
use feishu::events::SessionKey;
use feishu::messages::{ReceiveIdType, SendTextRequest};
use open_lark::Config as LarkConfig;
use open_lark::ws_client::{EventDispatcherHandler, EventHandler, LarkWsClient, WsClientError};
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

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

// 参数即 outbound 共享上下文（client/http/tokens/cfg/router/mgr/reactions），
// 打包 struct 只会给每个 match arm 增加 `ctx.` 噪音。
#[allow(clippy::too_many_arguments)]
async fn dispatch_out(
    feishu: &FeishuClient,
    http: &reqwest::Client,
    tokens: &feishu::client::TokenManager,
    cfg: &Config,
    router: &RouterHandle,
    mgr: &Arc<SessionManager>,
    reactions: &ReactionTracker,
    out: Out,
) -> anyhow::Result<()> {
    match out {
        Out::SendCard {
            key,
            card,
            msg_id,
            perm_request_id,
            perm_meta,
            root_id,
        } => {
            // The MsgIdMap is keyed by session_id (never chat_id) so that
            // `UpdateCard`/`React`, which only know the session_id, can resolve
            // the message_id. Only record when a session_id is supplied; plain
            // cards (permission prompts, help) don't need to be updated later.
            let new_id = feishu
                .send_card(http, tokens, &key, card, root_id.as_deref())
                .await?;
            if let (false, Some(session_id)) = (new_id.is_empty(), msg_id) {
                router.record_root_msg_id(session_id, new_id.clone()).await;
                debug!(message_id = %new_id, "recorded card msg_id");
            }
            // Permission cards are tracked by request_id so a later button
            // click can flip them in place (resolved/expired). We also stash
            // (tool_name, args) so "Allow session" can register an entry in
            // the session allowlist for auto-approving future calls.
            if let (false, Some(req_id)) = (new_id.is_empty(), perm_request_id) {
                let (tool_name, args) = perm_meta.unwrap_or_default();
                router
                    .record_perm_card_msg_id(
                        req_id.clone(),
                        key.clone(),
                        new_id.clone(),
                        tool_name,
                        args,
                    )
                    .await;
                debug!(%req_id, message_id = %new_id, "recorded perm card msg_id");
            }
        }
        Out::UpdateCard { session_id, card } => {
            if let Some(message_id) = router.root_msg_id(&session_id).await {
                feishu.update_card(http, tokens, &message_id, card).await?;
            } else {
                debug!(?session_id, "no root msg_id recorded; skipping update");
            }
        }
        Out::UpdateCardByMsgId { key, msg_id, card } => {
            // PATCH the card by its Feishu message_id (no session lookup
            // needed). Used for permission-card click feedback where the
            // session is still alive but we just want to flip the prompt in
            // place. Failure is non-fatal: a stale msg_id is a no-op on
            // Feishu's side.
            if let Err(e) = feishu.update_card(http, tokens, &msg_id, card).await {
                warn!(%msg_id, error=%e, "perm card update failed");
            }
            let _ = key; // chat context — currently unused; the API only needs msg_id
        }
        Out::React { session_id, emoji } => {
            if let Some(message_id) = router.root_msg_id(&session_id).await {
                match reactions.plan(&session_id, &emoji).await {
                    ReactPlan::Skip => {}
                    ReactPlan::ReactOnly => {
                        let rid = feishu.react(http, tokens, &message_id, &emoji).await?;
                        reactions.record(&session_id, emoji, rid).await;
                    }
                    ReactPlan::Swap { unreact_id } => {
                        // Best-effort: a stale reaction already gone is not fatal,
                        // but failing to add the new one would strand the state.
                        if let Err(e) = feishu.unreact(http, tokens, &message_id, &unreact_id).await
                        {
                            warn!(%session_id, "unreact before swap failed (continuing): {e}");
                        }
                        let rid = feishu.react(http, tokens, &message_id, &emoji).await?;
                        reactions.record(&session_id, emoji, rid).await;
                    }
                }
            } else {
                debug!(?session_id, "no root msg_id recorded; skipping react");
            }
        }
        Out::SpawnAcp { key, prompt } => {
            let claude = &cfg.acp.claude;
            // 1) Spawn the claude subprocess, mint a session_id, send the
            //    initial prompt, and flip the router's Spawning placeholder
            //    to Active (draining queued prompts). On failure (missing
            //    binary, handshake timeout, or prompt send failure): drop the
            //    placeholder and show an ❌ card instead of a silent log line
            //    (spec §4.1 "ACP spawn failure"). create_session and
            //    CreateSession-prompt failures share the same Err branch —
            //    both mean the session is unusable.
            let (session_id, pending, rx) = match acp_spawn_and_activate(
                mgr,
                router,
                &key,
                &prompt,
                &claude.path,
                claude.args.clone(),
                claude.work_dir.clone(),
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    router.fail_spawn(&key).await;
                    let card = feishu::cards::render_error_card(&format!(
                        "agent 启动失败/超时：{e}。请检查 claude 是否安装、PATH 是否正确。"
                    ));
                    if let Err(e2) = feishu
                        .send_card(http, tokens, &key, serde_json::to_value(&card)?, None)
                        .await
                    {
                        warn!(?e2, "failed to send spawn-failure card");
                    }
                    warn!(?e, "create_session failed");
                    return Ok(());
                }
            };
            wire_session_card_and_pump(
                feishu, http, tokens, cfg, router, mgr, reactions, key, session_id, prompt,
                pending, rx,
            )
            .await?;
        }
        Out::SpawnResume {
            key,
            session_id: old_sid,
            prompt,
        } => {
            let claude = &cfg.acp.claude;
            // Lazy respawn of a restored mapping (spec §3.3e): claude-native
            // `resume` of the persisted id; the manager transparently falls
            // back to a fresh session when the conversation is gone
            // (sebas-dk8.4). `resumed` says which happened — on fallback the
            // old context did NOT carry over, so tell the user instead of
            // silently continuing fresh.
            let (session_id, pending, rx, resumed) = match acp_resume_and_activate(
                mgr,
                router,
                &key,
                &old_sid,
                &prompt,
                &claude.path,
                claude.args.clone(),
                claude.work_dir.clone(),
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    router.fail_spawn(&key).await;
                    let card = feishu::cards::render_error_card(&format!(
                        "agent 恢复失败/超时：{e}。请检查 claude 是否安装、PATH 是否正确。"
                    ));
                    if let Err(e2) = feishu
                        .send_card(http, tokens, &key, serde_json::to_value(&card)?, None)
                        .await
                    {
                        warn!(?e2, "failed to send resume-failure card");
                    }
                    warn!(?e, %old_sid, "resume_session failed");
                    return Ok(());
                }
            };
            if !resumed {
                info!(%old_sid, %session_id, "old session could not be loaded; continued as fresh session");
                let card = feishu::cards::render_session_lost_card();
                if let Err(e2) = feishu
                    .send_card(http, tokens, &key, serde_json::to_value(&card)?, None)
                    .await
                {
                    warn!(?e2, "failed to send session-lost notice");
                }
            }
            wire_session_card_and_pump(
                feishu, http, tokens, cfg, router, mgr, reactions, key, session_id, prompt,
                pending, rx,
            )
            .await?;
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

/// Create the ACP session, send the initial prompt, and flip the router's
/// Spawning placeholder to Active (draining queued prompts). No Feishu side
/// effects, no event pump, no pending flush — the caller sequences those so
/// the root card can go out before the pump starts. Returns a clone of the
/// session's event receiver taken BEFORE the initial prompt is sent, so a
/// crash-on-first-prompt terminal event survives the wrapper's eager table
/// removal (D6). `pub` for integration tests (tests/spawn_race_test.rs);
/// not part of the stable API.
pub async fn acp_spawn_and_activate(
    mgr: &Arc<SessionManager>,
    router: &RouterHandle,
    key: &SessionKey,
    prompt: &str,
    claude_path: &str,
    claude_args: Vec<String>,
    work_dir: Option<String>,
) -> anyhow::Result<(
    String,
    Vec<String>,
    std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<acp_claude::session::AcpEvent>>>,
)> {
    let session_id = mgr
        .create_session(claude_path, claude_args, work_dir, prompt.to_string())
        .await?;
    // Clone the event receiver IMMEDIATELY after create_session returns Ok
    // (entry is in the table, session alive — before any slow I/O or prompt
    // send). This guarantees that if the agent crashes on the first prompt
    // (D6 target), the buffered terminal event survives the wrapper's eager
    // table removal — the dropped entry only releases the manager's Arc
    // clone, not this consumer's.
    let rx = mgr
        .event_rx(&session_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("no event_rx for freshly-created session"))?;
    mgr.send(
        &session_id,
        AcpCommand::CreateSession {
            session_id: session_id.clone(),
            prompt: prompt.to_string(),
        },
    )
    .await?;
    let pending = router.activate(key, session_id.clone()).await;
    Ok((session_id, pending, rx))
}

/// Resume variant of [`acp_spawn_and_activate`] (spec §3.3e): ask claude to
/// `resume` the persisted conversation id (the manager transparently falls
/// back to a fresh session when the id is rejected — sebas-dk8.4), then push
/// the triggering prompt as a continuation and flip the router's placeholder
/// to Active. The returned bool is `SpawnOutcome.resumed` — false means the
/// old conversation is gone and the id is a fresh one. The event receiver is
/// cloned IMMEDIATELY after the spawn returns, before any slow I/O (D6).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub async fn acp_resume_and_activate(
    mgr: &Arc<SessionManager>,
    router: &RouterHandle,
    key: &SessionKey,
    old_session_id: &str,
    prompt: &str,
    claude_path: &str,
    claude_args: Vec<String>,
    work_dir: Option<String>,
) -> anyhow::Result<(
    String,
    Vec<String>,
    std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<acp_claude::session::AcpEvent>>>,
    bool,
)> {
    let outcome = mgr
        .resume_session(claude_path, claude_args, work_dir, old_session_id)
        .await?;
    let session_id = outcome.session_id.clone();
    let rx = mgr
        .event_rx(&session_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("no event_rx for freshly-resumed session"))?;
    // The triggering prompt rides as a continuation: for a resumed session
    // it appends to the loaded conversation; for a fallback-fresh session
    // it is simply the first prompt (run_main drives both through
    // `send_prompt`).
    mgr.send(
        &session_id,
        AcpCommand::ContinueSession {
            session_id: session_id.clone(),
            prompt: prompt.to_string(),
        },
    )
    .await?;
    let pending = router.activate(key, session_id.clone()).await;
    Ok((session_id, pending, rx, outcome.resumed))
}

/// Restore the session map from the state file (spec §3.3e):
/// - missing or empty file → empty table (first boot; an empty file is a
///   harmless leftover, not corruption);
/// - valid JSON → entries come back `Dormant`, eligible for lazy respawn;
/// - corrupt JSON → quarantine the file to `<path>.corrupt-<unix>` and
///   boot with an empty table instead of refusing to start.
///
/// `capacity` wires `[router] max_concurrent_sessions` into the map.
pub fn restore_session_map(state_file: &str, capacity: usize) -> SessionMap {
    let state_raw = std::fs::read_to_string(state_file).unwrap_or_else(|_| "{}".into());
    match state_raw.trim() {
        "" => SessionMap::with_capacity(capacity),
        raw => match SessionMap::restore_json_with_capacity(raw, capacity) {
            Ok(m) => m,
            Err(e) => {
                let unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let quarantined = format!("{state_file}.corrupt-{unix}");
                warn!(
                    ?e,
                    path = %state_file,
                    "session state file is corrupt; quarantining and starting fresh"
                );
                if let Err(re) = std::fs::rename(state_file, &quarantined) {
                    warn!(?re, path = %quarantined, "failed to quarantine corrupt state file");
                }
                SessionMap::with_capacity(capacity)
            }
        },
    }
}

/// Shared post-spawn wiring for the SpawnAcp and SpawnResume dispatch arms:
/// seed the card state, send the root card, record its message_id, start the
/// event pump, and flush prompts queued during the spawn.
#[allow(clippy::too_many_arguments)]
async fn wire_session_card_and_pump(
    feishu: &FeishuClient,
    http: &reqwest::Client,
    tokens: &feishu::client::TokenManager,
    cfg: &Config,
    router: &RouterHandle,
    mgr: &Arc<SessionManager>,
    reactions: &ReactionTracker,
    key: SessionKey,
    session_id: String,
    prompt: String,
    pending: Vec<String>,
    rx: std::sync::Arc<
        tokio::sync::Mutex<tokio::sync::mpsc::Receiver<acp_claude::session::AcpEvent>>,
    >,
) -> anyhow::Result<()> {
    // seed_card（spec §4.2）: 记录 user_prompt 供后续 flush 重渲染
    // 引用块。幂等。必须在 pump 启动前，否则首个事件 lazy seed
    // 会用 prompt="" 冲掉引用块。
    router.seed_card(session_id.clone(), prompt.clone()).await;
    // Send the seed card (empty body) and record its message_id keyed by the
    // real session_id (so streaming UpdateCards resolve correctly).
    // render_accumulated_card 用真实 theme，与后续 flush 产出的卡结构一致
    //（避免初始卡蓝、后续卡变色的跳变）。
    // status_emoji 在 state 里是 Feishu emoji_type（"Typing"），渲染时通过
    // phase_visual 映射成 💬。
    let seed_emoji = feishu::cards::phase_visual(router::card_state::phase::SEED);
    let card =
        render_accumulated_card(&prompt, &session_id, seed_emoji, &[], &cfg.card.theme_color);
    let msg_id = feishu
        .send_card(http, tokens, &key, serde_json::to_value(&card)?, None)
        .await?;
    if !msg_id.is_empty() {
        router
            .record_root_msg_id(session_id.clone(), msg_id.clone())
            .await;
        // Stamp the initial reaction on the root card. emoji_type 是 Feishu
        // API 合法值（"Typing"），不再是 unicode 👀 —— 那个会被 231001 拒绝。
        // Best-effort: a reaction failure must not abort session creation.
        match feishu
            .react(http, tokens, &msg_id, router::card_state::phase::SEED)
            .await
        {
            Ok(rid) => {
                reactions
                    .record(&session_id, router::card_state::phase::SEED.into(), rid)
                    .await
            }
            Err(e) => warn!(%session_id, "initial react failed: {e}"),
        }
    }
    // Pump ACP events from this session back into the router.
    // `rx` was cloned before any slow I/O (the send_card HTTP round trip
    // above) so a crash-on-first-prompt terminal event survives the
    // wrapper's eager table removal (D6).
    spawn_acp_pump(rx, router.clone(), session_id.clone());
    // Flush queued prompts as ONE follow-up (sending them one by one would
    // violate ACP's one-prompt-in-flight rule).
    if let Err(e) = flush_pending_prompts(mgr, &session_id, pending).await {
        warn!(?e, "failed to flush pending prompts");
    }
    Ok(())
}

/// Flush prompts queued during spawn as ONE ContinueSession (one-by-one
/// would violate ACP's one-prompt-in-flight rule). `pub` for tests.
pub async fn flush_pending_prompts(
    mgr: &Arc<SessionManager>,
    session_id: &str,
    pending: Vec<String>,
) -> anyhow::Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    mgr.send(
        session_id,
        AcpCommand::ContinueSession {
            session_id: session_id.to_string(),
            prompt: pending.join("\n"),
        },
    )
    .await
}

/// Drain ACP events for one session, accumulating them into CardState and
/// flushing a single UpdateCard at most once per 150 ms (spec §6 节流契约).
///
/// - 流式事件（TextDelta/ThinkingDelta/ToolStart/ToolProgress/ToolEnd/非
///   terminal Error）: `apply_event`（状态）+ 标脏；interval tick 到点若脏
///   则 `flush_card`。FSM 转移出的 reaction 随 flush 一起发（`pending_react`），
///   保持「先出卡、后换 reaction」的顺序。
/// - Finished / terminal Error / PermissionRequest: 即时 `apply_event_to_out`
///   （terminal 额外 remove_by_session + drop_card 后泵退出）。该路径自带
///   即时 React，故丢弃未发的 `pending_react`（避免 ✅/❌ 之后又补一个 🚧）。
/// - 通道关闭（recv → None）: `drop_card` + 退出。
///
/// `rx` 在 `acp_spawn_and_activate` 里于任何慢 I/O 之前克隆，故即便 agent
/// 首次 prompt 即崩（D6）、wrapper 急切移除表项，终端事件仍能经此克隆抵达。
///
/// 机制选择（spec §6 把 async 机制委托给计划钉死）：用
/// `tokio::time::interval(150ms) + dirty bool`，而非 spec 建议的
/// `Option<Sleep> + select + pending()` —— 后者在 select 跨臂借用 `&mut`
/// 会冲突，interval + Copy bool 规避之，契约等价。
pub fn spawn_acp_pump(
    rx: std::sync::Arc<
        tokio::sync::Mutex<tokio::sync::mpsc::Receiver<acp_claude::session::AcpEvent>>,
    >,
    router: RouterHandle,
    session_id: String,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(150));
        // 第一个 tick 立即触发（tokio interval 语义）；此时 dirty=false，是 no-op。
        let mut dirty = false;
        let mut pending_react: Option<&'static str> = None;
        let mut rx = rx.lock().await;
        loop {
            tokio::select! {
                maybe_evt = rx.recv() => {
                    let Some(evt) = maybe_evt else {
                        router.drop_card(&session_id).await;
                        break;
                    };
                    let is_terminal = matches!(evt, AcpEvent::Error { terminal: true, .. });
                    let is_immediate = matches!(
                        evt,
                        AcpEvent::Finished { .. }
                            | AcpEvent::Error { terminal: true, .. }
                            | AcpEvent::PermissionRequest { .. }
                    );
                    if is_immediate {
                        // 即时路径：取消待发 debounce，同步出最终态。
                        dirty = false;
                        pending_react = None;
                        router.apply_event_to_out(session_id.clone(), &evt).await;
                        if is_terminal {
                            break;
                        }
                    } else {
                        // 流式：只累积状态，标脏；FSM 转移（👀→🚧）的 reaction
                        // 记入 pending_react，随下次 flush 一起发。
                        if let Some(emoji) = router.apply_event(&session_id, &evt).await {
                            pending_react = Some(emoji);
                        }
                        dirty = true;
                    }
                }
                _ = ticker.tick() => {
                    if dirty {
                        dirty = false;
                        router.flush_card(&session_id).await;
                    }
                    if let Some(emoji) = pending_react.take() {
                        router.emit_reaction(&session_id, emoji).await;
                    }
                }
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
/// Note on event coverage: v0.19.0's `register_raw` accepts any non-empty
/// string key, so we register both inbound event names we care about:
/// `im.message.receive_v1` (text/media) and `card.action.trigger` (button
/// callbacks from permission cards). Each registration hands the same
/// `RouterEventHandler` clone every frame; the handler parses both
/// envelope shapes via `FeishuEnvelope::into_event`.
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
        // Two raw registrations sharing the same handler. `register_raw` in
        // v0.19.0 is purely a key-on-HashMap insert keyed by the supplied
        // string, so `card.action.trigger` is accepted — there is no enum
        // of supported events on the openlark side. Any registration error
        // (empty / duplicate key) is bubbled up and aborts the WS loop.
        let dispatcher = match EventDispatcherHandler::builder()
            .register_raw("im.message.receive_v1", handler.clone())
            .and_then(|b| b.register_raw("card.action.trigger", handler))
        {
            Ok(d) => d,
            Err(e) => {
                error!(
                    error = %e,
                    "failed to register event handlers; aborting WS loop"
                );
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

/// Raw-bytes event handler bound to inbound event names via `register_raw`.
/// Bypasses v0.14.0's typed-registration bug (where the dispatcher built the
/// lookup key as `schema.type_` instead of the server-emitted `p2.*` key,
/// dropping every inbound message) by avoiding the typed dispatch layer
/// entirely: we get the framed JSON payload, parse it as our own
/// `FeishuEnvelope`, and forward into the router. The same instance is
/// registered twice (`im.message.receive_v1` for text/media and
/// `card.action.trigger` for permission-card button callbacks), so the
/// struct must be cheap to clone — all owned fields are already `Clone`.
///
/// Also reused by `crate::replay` (the `sebas replay --dir` subcommand) so
/// the WS path and the offline replay path share the same parse + dispatch
/// logic 1:1. Fields are `pub` so tests and `replay::run` can construct one
/// directly without a constructor.
#[derive(Clone)]
pub struct RouterEventHandler {
    pub router: RouterHandle,
    pub owner_id: String,
    /// Optional directory for raw payload snapshots. When set, every received
    /// WS frame is written to `<dir>/<unix_ms>-<uuid>.json` before parsing, so
    /// you can replay captured traffic locally without a live Feishu bot.
    pub dump_dir: Option<std::path::PathBuf>,
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
        // Delegate the parse + dispatch to the shared replay helper so the
        // WS loop and `sebas replay --dir` exercise the exact same routing
        // logic. `replay_frame` is sync; it spawns the async dispatch.
        crate::replay::replay_frame(self, payload);
        Ok(())
    }
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

/// Test-only helper used by the SIGTERM-cleanup integration test
/// (`tests/sigterm_cleanup_test.rs`). Spawns one ACP session against the
/// configured `acp.claude.path` and records a synthetic `SessionKey` in
/// the router, so a child process is alive as a descendant of the sebas
/// pid by the time SIGTERM arrives. Production callers never set
/// `SEBAS_TEST_SPAWN_SESSION`, so this path is dormant.
async fn spawn_test_session(cfg: &Config, router: &RouterHandle, mgr: &SessionManager) {
    let claude = &cfg.acp.claude;
    let session_id = match mgr
        .create_session(
            &claude.path,
            claude.args.clone(),
            claude.work_dir.clone(),
            "[test-mode] sigterm-cleanup probe".into(),
        )
        .await
    {
        Ok(id) => id,
        Err(e) => {
            warn!(?e, "SEBAS_TEST_SPAWN_SESSION: create_session failed");
            return;
        }
    };
    // Forward the initial prompt via the command channel so the child
    // transitions out of `session/new` and into the read loop. Without
    // this, fake-claude would block waiting for a prompt and we'd never
    // see any child liveness signal.
    if let Err(e) = mgr
        .send(
            &session_id,
            AcpCommand::CreateSession {
                session_id: session_id.clone(),
                prompt: "[test-mode] sigterm-cleanup probe".into(),
            },
        )
        .await
    {
        warn!(?e, "SEBAS_TEST_SPAWN_SESSION: send failed");
    }
    // Synthetic SessionKey — the test never sends a real Feishu message,
    // so the key content doesn't matter; it just needs to be unique.
    let key = SessionKey {
        chat_id: format!("test-sigterm-{}", std::process::id()),
        thread_id: None,
    };
    router.insert_mapping(key, session_id.clone()).await;
    info!(%session_id, "SEBAS_TEST_SPAWN_SESSION: spawned child session");
}
