//! 飞书 WebSocket 长连接事件循环 + 入站事件处理器。
//!
//! 从 `run.rs` 拆出。`RouterEventHandler` 同时被 `crate::replay` 复用
//! （WS 路径与离线 replay 共享同一套解析+分发），经 `crate::run` re-export。

use crate::config::Config;
use acp_claude::manager::SessionManager;
use acp_claude::session::AcpCommand;
use feishu::events::FeishuIn;
use feishu::events::SessionKey;
use open_lark::Config as LarkConfig;
use open_lark::CoreError;
use open_lark::ws_client::{EventDispatcherHandler, EventHandler, LarkWsClient, WsClientError};
use router::router::RouterHandle;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tracing::{error, info, warn};

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
pub(crate) async fn run_ws_loop(
    app_id: &str,
    app_secret: &str,
    owner_id: &str,
    router: RouterHandle,
    dump_dir: Option<std::path::PathBuf>,
    allowed_chat_types: Vec<String>,
    bot_name: String,
) {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(60);

    loop {
        // Rebuild the dispatcher for each connection attempt so retries start
        // with a fresh handler and cheap clones of the router and owner ID.
        let mut handler =
            RouterEventHandler::new(router.clone(), owner_id.to_string(), dump_dir.clone());
        // Wire chat_type/bot_name filter from config (pub fields)
        handler.allowed_chat_types = allowed_chat_types.clone();
        handler.bot_name = bot_name.clone();
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
            Err(WsClientError::RequestError(core_err))
                if matches!(core_err, CoreError::Authentication { .. }) =>
            {
                error!(
                    error = %core_err,
                    "feishu WS auth failed; aborting (fatal)"
                );
                return;
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
    /// 已处理过的 event_id 集合（去重）。飞书可能重投相同 event_id 的事件。
    /// 容量上限 4096，超限时整体清空（概率极低，但防内存泄漏）。
    pub seen_events: Arc<Mutex<HashSet<String>>>,
    /// 允许的 chat_type 列表（"private", "group" 等）。空列表 = 全部允许。
    pub allowed_chat_types: Vec<String>,
    /// 机器人名称（用于群聊 @ 检测）。仅在 group chat_type 时检查。
    /// 空字符串 = 不检查 @。
    pub bot_name: String,
}

/// chat_type 归一化："private"（本地缺省/存量配置的幻影值）映射到飞书
/// 真实私聊 wire 值 "p2p"，其余原样返回。
fn norm_chat_type(t: &str) -> &str {
    if t == "private" { "p2p" } else { t }
}

impl RouterEventHandler {
    pub fn new(
        router: RouterHandle,
        owner_id: String,
        dump_dir: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            router,
            owner_id,
            dump_dir,
            seen_events: Arc::new(Mutex::new(HashSet::new())),
            allowed_chat_types: Vec::new(),
            bot_name: String::new(),
        }
    }

    /// 是否允许该 chat_type 的消息。
    /// 空列表 = 全部允许。
    /// "private" ↔ "p2p" 视为同值（sebas-5y5）：飞书私聊真实 wire 值是
    /// "p2p"，"private" 只出现在本地缺省/存量配置里，两侧归一化后比较。
    pub fn is_chat_type_allowed(&self, chat_type: &str) -> bool {
        self.allowed_chat_types.is_empty()
            || self
                .allowed_chat_types
                .iter()
                .any(|t| norm_chat_type(t) == norm_chat_type(chat_type))
    }

    /// 检查消息是否需要 @bot 过滤。
    /// 群聊（group/p2p）中非 @bot 消息应过滤。
    pub fn should_filter_by_mention(&self, evt: &FeishuIn) -> bool {
        // 当前只处理私聊/群聊文本消息的 @ 过滤
        let chat_type = evt.chat_type();
        if chat_type != "group" && chat_type != "p2p" {
            return false; // 私聊不过滤
        }
        // 无 bot_name 配置时不过滤
        if self.bot_name.is_empty() {
            return false;
        }
        // 检查 mentions 列表中是否包含 bot 名称
        let mentioned = evt.mentions().iter().any(|m| {
            m.name
                .to_lowercase()
                .contains(&self.bot_name.to_lowercase())
                || m.key.to_lowercase().contains(&self.bot_name.to_lowercase())
        });
        !mentioned
    }
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

/// Test-only helper used by the SIGTERM-cleanup integration test
/// (`tests/sigterm_cleanup_test.rs`). Spawns one ACP session against the
/// configured `acp.claude.path` and records a synthetic `SessionKey` in
/// the router, so a child process is alive as a descendant of the sebas
/// pid by the time SIGTERM arrives. Production callers never set
/// `SEBAS_TEST_SPAWN_SESSION`, so this path is dormant.
pub(crate) async fn spawn_test_session(cfg: &Config, router: &RouterHandle, mgr: &SessionManager) {
    let claude = &cfg.acp.claude;
    let session_id = match mgr
        .create_session(
            &claude.path,
            claude.args.clone(),
            claude.work_dir.clone(),
            Vec::new(), // no provider-mode env in test harness
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
