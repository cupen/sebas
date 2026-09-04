//! `sebas replay --dir <DIR>` — offline replay of captured inbound events.
//!
//! Reads every `*.json` file in the directory (sorted by filename), parses
//! each as a neutral [`ChannelEvent`] (the post-adapter, post-gating shape
//! the live WS loop dumps), and dispatches into a fresh `RouterHandle`.
//! No Feishu client, no ACP child, no HTTP server — this is a pure
//! router/manager exercise used to reproduce router decisions and
//! `apply_event_to_out` actions offline.
//!
//! Frame format: externally-tagged `ChannelEvent` JSON
//! (`{"Text": {"key": {"channel": "feishu", "reference": "..."}, ...}}`).
//! Captured events have already passed the adapter's gates (dedup,
//! chat-type, mention) at capture time, so replay re-applies none of them.
//! Pre-neutralization fixture dumps (raw Feishu envelopes) no longer parse
//! and are skipped with a warning (decouple-feishu-channel design D6).

use std::path::PathBuf;

use sebas_channels::ChannelEvent;
use sebas_router::router::RouterHandle;
use sebas_router::state::SessionMap;
use tracing::{debug, info, warn};

/// Arguments for `sebas replay --dir <DIR>`.
pub struct ReplayArgs {
    pub dir: PathBuf,
}

/// Replay ONE neutral frame synchronously against the router.
///
/// Synchronous so tests (and any sync caller) can drive it; the actual
/// `dispatch` call is async, so we `tokio::spawn` it and return immediately.
///
/// Returns `true` if the frame was parsed and dispatched to the router,
/// `false` if it was skipped (UTF-8 / JSON parse failure).
pub fn replay_frame(router: &RouterHandle, raw: &[u8]) -> bool {
    let text = match std::str::from_utf8(raw) {
        Ok(t) => t,
        Err(e) => {
            warn!(?e, "replay: non-UTF8 payload, skipping");
            return false;
        }
    };
    let evt = match serde_json::from_str::<ChannelEvent>(text) {
        Ok(e) => e,
        Err(e) => {
            warn!(?e, "replay: failed to parse ChannelEvent, skipping");
            return false;
        }
    };
    let router = router.clone();
    // Dispatch is async; the caller (tests or replay::run) is sync. Spawn
    // and let the runtime drive it. The mpsc channel inside the router
    // absorbs the result so the caller does not need to await.
    tokio::spawn(async move {
        router.dispatch(evt).await;
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
        let dispatched = replay_frame(&router, &bytes);
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
