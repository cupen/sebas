//! 飞书入站事件处理器（feishu 适配器边界）。
//!
//! decouple-feishu-channel task 3/4：生产 WS 循环已下沉到 feishu adapter
//! （`sebas_feishu::adapter::FeishuAdapter::spawn` → 内部 `run_ws_loop`）。
//! 这里保留 `DispatchEventHandler` + `feishu_in_to_channel_event` +
//! `ingest_feishu_frame`：envelope 解析、去重、门禁、中立化翻译都在这一层
//! 完成，dump 的是翻译后的中立 `ChannelEvent`（`sebas replay` 消费同一
//! 形状，replay 侧零飞书引用）。

use crate::config::Config;
use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::AcpCommand;
use sebas_feishu::events::{FeishuEnvelope, FeishuIn};
use open_lark::ws_client::EventHandler;
use sebas_dispatch::engine::DispatchHandle;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use tracing::{debug, info, warn};

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
/// Fields are `pub` so tests and the adapter wiring can construct one
/// directly without a constructor.
#[derive(Clone)]
pub struct DispatchEventHandler {
    pub router: DispatchHandle,
    pub owner_id: String,
    /// Optional directory for **translated neutral event** snapshots. When set,
    /// every gated inbound frame is written as `ChannelEvent` JSON to
    /// `<dir>/<unix_ns>-<pid>.json`, so `sebas replay` can replay captured
    /// traffic locally without a live Feishu bot.
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

impl DispatchEventHandler {
    pub fn new(
        router: DispatchHandle,
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

impl EventHandler for DispatchEventHandler {
    fn handle(
        &self,
        payload: &[u8],
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Feishu-boundary ingest: envelope parse → dedup → gates → neutral
        // translation → dump (neutral) → dispatch. `ingest_feishu_frame` is
        // sync; it spawns the async dispatch.
        ingest_feishu_frame(self, payload);
        Ok(())
    }
}

/// Feishu-boundary ingest for one raw WS frame (also drives the envelope-
/// driven tests): parse the `FeishuEnvelope`, dedup by `event_id`, apply the
/// chat-type/mention gates, translate to a neutral [`ChannelEvent`], dump
/// the **translated** event (so `sebas replay` consumes neutral frames), and
/// dispatch into the router.
///
/// Synchronous so the WS handler and tests can call it; the actual dispatch
/// is async and spawned. Returns `true` if a frame was translated and
/// dispatched, `false` if it was skipped (parse failure, owner filter,
/// gates, or unrecognized envelope).
pub fn ingest_feishu_frame(handler: &DispatchEventHandler, raw: &[u8]) -> bool {
    let text = match std::str::from_utf8(raw) {
        Ok(t) => t,
        Err(e) => {
            warn!(?e, "ingest: non-UTF8 payload, skipping");
            return false;
        }
    };
    let env = match serde_json::from_str::<FeishuEnvelope>(text) {
        Ok(e) => e,
        Err(e) => {
            warn!(?e, "ingest: failed to parse FeishuEnvelope, skipping");
            return false;
        }
    };

    // 事件去重：飞书可能重投相同 event_id 的事件。
    if let Some(ref eid) = env.header.event_id {
        let mut seen = handler.seen_events.lock().unwrap();
        if !seen.insert(eid.clone()) {
            debug!(event_id = %eid, "ingest: duplicate event, skipping");
            return false;
        }
        // 容量上限 4096，超限时整体清空。
        if seen.len() > 4096 {
            seen.clear();
        }
    }

    let Some(in_ev) = env.into_event(&handler.owner_id) else {
        debug!("ingest: envelope produced no FeishuIn (filtered or unrecognized)");
        return false;
    };

    // chat_type 过滤 + 群聊 @bot 检测
    if !handler.is_chat_type_allowed(in_ev.chat_type()) {
        debug!("ingest: chat_type not allowed, skipping");
        return false;
    }
    if handler.should_filter_by_mention(&in_ev) {
        debug!("ingest: group message without @bot, skipping");
        return false;
    }

    // 中立化：飞书入站事件 → ChannelEvent 再进 router（decouple-feishu-channel）。
    let channel_evt =
        sebas_feishu::adapter::feishu_in_to_channel_event(in_ev);

    // Dump the **translated** neutral event so `sebas replay` consumes the
    // same shape offline (post-gates, post-translation).
    if let Some(dir) = &handler.dump_dir {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let path = dir.join(format!("{ts}-{pid}.json"));
        match serde_json::to_vec(&channel_evt) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    warn!(?e, ?path, "failed to dump inbound channel event");
                }
            }
            Err(e) => warn!(?e, "failed to serialize channel event for dump"),
        }
    }

    let router = handler.router.clone();
    // Dispatch is async; the caller (WS handler or tests) is sync. Spawn and
    // let the runtime drive it. The mpsc channel inside the router absorbs
    // the result so the caller does not need to await.
    tokio::spawn(async move {
        router.dispatch(channel_evt).await;
    });
    debug!("ingest: dispatched frame");
    true
}

/// Test-only helper used by the SIGTERM-cleanup integration test
/// (`tests/sigterm_cleanup_test.rs`). Spawns one ACP session against the
/// configured `acp.claude.path` and records a synthetic `SessionKey` in
/// the router, so a child process is alive as a descendant of the sebas
/// pid by the time SIGTERM arrives. Production callers never set
/// `SEBAS_TEST_SPAWN_SESSION`, so this path is dormant.
pub(crate) async fn spawn_test_session(cfg: &Config, router: &DispatchHandle, mgr: &SessionManager) {
    let kind = cfg.acp.default_kind().to_string();
    let command = cfg.acp.command_for(&kind).unwrap_or_default();
    let session_id = match mgr
        .create_session(
            &kind,
            command,
            cfg.acp.work_dir_for(&kind),
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
    let key = sebas_channels::ChannelKey::feishu(
        &format!("test-sigterm-{}", std::process::id()),
        None,
    );
    router.insert_mapping(key, session_id.clone()).await;
    info!(%session_id, "SEBAS_TEST_SPAWN_SESSION: spawned child session");
}
