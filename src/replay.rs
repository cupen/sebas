//! `sebas replay --dir <DIR>` — offline replay of captured inbound events.
//!
//! Reads every `*.json` file in the directory (sorted by filename), parses
//! each as a `FeishuEnvelope`, and dispatches into a fresh `RouterHandle`
//! exactly the same way the live Feishu WebSocket loop would. No Feishu
//! client, no ACP child, no HTTP server — this is a pure router/manager
//! exercise used to reproduce router decisions and `apply_event_to_out`
//! actions offline.
//!
//! The parse + dispatch path is shared with `crate::run::RouterEventHandler`
//! via `replay_frame`, so the WS handler and the replay command exercise
//! the same routing code 1:1.

use std::path::PathBuf;

use feishu::events::FeishuEnvelope;
use router::router::RouterHandle;
use router::state::SessionMap;
use tracing::{debug, info, warn};

use crate::run::RouterEventHandler;

/// Arguments for `sebas replay --dir <DIR>`.
pub struct ReplayArgs {
    pub dir: PathBuf,
}

/// Replay ONE frame synchronously against the dispatcher.
///
/// Synchronous so it can be called from `EventHandler::handle` (the WS
/// handler runs in sync context) and from tests; the actual `dispatch`
/// call is async, so we `tokio::spawn` it and return immediately.
///
/// Returns `true` if the frame was parsed and dispatched to the router,
/// `false` if it was skipped (UTF-8 / JSON parse failure, or the envelope
/// mapped to no internal event — e.g. owner filter excluded the sender).
pub fn replay_frame(handler: &RouterEventHandler, raw: &[u8]) -> bool {
    let text = match std::str::from_utf8(raw) {
        Ok(t) => t,
        Err(e) => {
            warn!(?e, "replay: non-UTF8 payload, skipping");
            return false;
        }
    };
    let env = match serde_json::from_str::<FeishuEnvelope>(text) {
        Ok(e) => e,
        Err(e) => {
            warn!(?e, "replay: failed to parse FeishuEnvelope, skipping");
            return false;
        }
    };

    // 事件去重：飞书可能重投相同 event_id 的事件。
    if let Some(ref eid) = env.header.event_id {
        let mut seen = handler.seen_events.lock().unwrap();
        if !seen.insert(eid.clone()) {
            debug!(event_id = %eid, "replay: duplicate event, skipping");
            return false;
        }
        // 容量上限 4096，超限时整体清空。
        if seen.len() > 4096 {
            seen.clear();
        }
    }

    let Some(in_ev) = env.into_event(&handler.owner_id) else {
        debug!("replay: envelope produced no FeishuIn (filtered or unrecognized)");
        return false;
    };

    // chat_type 过滤 + 群聊 @bot 检测
    if !handler.is_chat_type_allowed(in_ev.chat_type()) {
        debug!("replay: chat_type not allowed, skipping");
        return false;
    }
    if handler.should_filter_by_mention(&in_ev) {
        debug!("replay: group message without @bot, skipping");
        return false;
    }

    let router = handler.router.clone();
    // Dispatch is async; the caller (WS handler or replay::run) is sync.
    // Spawn and let the runtime drive it. The mpsc channel inside the
    // router absorbs the result so the caller does not need to await.
    tokio::spawn(async move {
        router.dispatch(in_ev).await;
    });
    debug!("replay: dispatched frame");
    true
}

/// Run the replay flow: glob `dir/*.json`, parse, dispatch in order.
/// Returns the count of frames that were successfully dispatched.
///
/// Exits 1 (via `anyhow::Error`) when `dir` does not exist or `read_dir`
/// fails. Per-frame read errors and JSON parse failures are warn-and-skip
/// so a single bad file cannot abort the whole replay.
pub async fn run(args: ReplayArgs) -> anyhow::Result<u64> {
    if !args.dir.exists() {
        anyhow::bail!("dir not found: {}", args.dir.display());
    }

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&args.dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case("json"))
        })
        .collect();
    // Timestamp-prefixed filenames sort lexically by capture time, which
    // is the order replay should preserve.
    paths.sort();
    info!(
        "replay: found {} .json file(s) in {}",
        paths.len(),
        args.dir.display()
    );

    // Fresh router per replay invocation. The receiver is held in scope for
    // the duration of `run` so the spawned dispatch tasks can successfully
    // push — `mpsc::Sender::send` returns an error if all receivers are
    // dropped, which would silently swallow every frame.
    let (router, _rx) = RouterHandle::new(SessionMap::new());
    let handler = RouterEventHandler::new(
        router,
        String::new(),
        None,
    );

    let mut count: u64 = 0;
    for p in &paths {
        let bytes = match std::fs::read(p) {
            Ok(b) => b,
            Err(e) => {
                warn!(?e, path = %p.display(), "replay: failed to read file, skipping");
                tokio::task::yield_now().await;
                continue;
            }
        };
        let dispatched = replay_frame(&handler, &bytes);
        if dispatched {
            debug!(path = %p.display(), "replay: dispatched frame");
            count += 1;
        } else {
            debug!(path = %p.display(), "replay: skipped frame");
        }
        // Sequential replay: yield to let the spawned dispatch task run
        // before we read the next frame. Order matters (a Session binding
        // `text` then `button cb` should land in order), so we do NOT
        // parallelize.
        tokio::task::yield_now().await;
    }

    println!("replayed {count} frames from {}", args.dir.display());
    Ok(count)
}
