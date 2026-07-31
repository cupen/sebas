//! Workspace-level tests for the `sebas replay` subcommand.
//!
//! These tests exercise the shared `replay_frame` helper directly so the
//! FS read path is skipped — the test boundary is the parse + dispatch step
//! that both the live WS handler and the offline replay command share.

use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use sebas::replay::replay_frame;
use sebas::run::RouterEventHandler;
use tokio::sync::mpsc::Receiver;

/// Build a fresh `RouterEventHandler` with an empty `SessionMap` and a
/// captured `Out` receiver so the test can assert what was dispatched.
fn make_handler(owner_id: &str) -> (RouterEventHandler, Receiver<Out>) {
    let (router, rx) = RouterHandle::new(SessionMap::new());
    let handler = RouterEventHandler {
        router,
        owner_id: owner_id.to_string(),
        dump_dir: None,
    };
    (handler, rx)
}

/// Drain at most `max_yields` scheduler ticks, returning the first `Out`
/// the dispatcher produces (or `None` if nothing arrived). One yield is
/// usually enough, but the dispatch path makes several `.await` calls, so
/// we loop to be deterministic.
async fn recv_within(rx: &mut Receiver<Out>, max_yields: usize) -> Option<Out> {
    for _ in 0..max_yields {
        tokio::task::yield_now().await;
        if let Ok(out) = rx.try_recv() {
            return Some(out);
        }
    }
    None
}

#[tokio::test]
async fn replay_one_text_message_emits_spawn_acp() {
    let (handler, mut rx) = make_handler("ou_owner");
    let payload = serde_json::to_vec(&serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "im.message.receive_v1", "tenant_key": "t" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_owner" } },
            "message": {
                "chat_id": "oc_test",
                "message_id": "om_1",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}"
            }
        }
    }))
    .expect("serialize envelope");

    assert!(
        replay_frame(&handler, &payload),
        "replay_frame should accept a well-formed text envelope"
    );

    let out = recv_within(&mut rx, 64)
        .await
        .expect("expected Out::SpawnAcp after replay_frame");
    assert!(
        matches!(out, Out::SpawnAcp { .. }),
        "expected Out::SpawnAcp, got {out:?}"
    );
}

#[tokio::test]
async fn replay_button_cb_routes_to_help_when_session_dead() {
    let (handler, mut rx) = make_handler("ou_owner");
    let payload = serde_json::to_vec(&serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "card.action.trigger", "tenant_key": "t" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_owner" } },
            "chat_id": "oc_test",
            "action": {
                "session_id": "sess_missing",
                "request_id": "req_1",
                "decision": "allow_once"
            }
        }
    }))
    .expect("serialize envelope");

    assert!(
        replay_frame(&handler, &payload),
        "replay_frame should accept a well-formed card.action.trigger envelope"
    );

    let out = recv_within(&mut rx, 64)
        .await
        .expect("expected an Out after replay_frame for a dead-session button cb");

    // The current router routing for a button cb with no live session is
    // `Out::SendCard` carrying the dead-session card (see `on_button` in
    // `router/src/router.rs`). The brief also accepts `Out::HelpText` if
    // the routing branch is ever changed to emit help instead; both are
    // valid "button cb was dispatched" outcomes.
    assert!(
        matches!(out, Out::SendCard { .. } | Out::HelpText { .. }),
        "expected Out::SendCard (dead session) or Out::HelpText, got {out:?}"
    );
}

/// `replay::run` 的 FS 路径：目录 glob、按文件名排序、逐帧 dispatch、
/// 坏帧 warn-and-skip、dir 不存在 → Err。
#[tokio::test]
async fn replay_run_reads_sorted_frames_and_skips_bad_ones() {
    let dir = std::env::temp_dir().join(format!("sebas-replay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mk_frame = |chat: &str| {
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "2.0",
            "header": { "event_type": "im.message.receive_v1", "tenant_key": "t" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_anyone" } },
                "message": {
                    "chat_id": chat,
                    "message_id": "om_1",
                    "message_type": "text",
                    "content": "{\"text\":\"hi\"}"
                }
            }
        }))
        .unwrap()
    };
    // 时间戳前缀保证字典序 = 捕获序；混入一个坏帧和一个非 .json 文件。
    std::fs::write(dir.join("001-a.json"), mk_frame("oc_a")).unwrap();
    std::fs::write(dir.join("002-b.json"), mk_frame("oc_b")).unwrap();
    std::fs::write(dir.join("003-bad.json"), "{ nope").unwrap();
    std::fs::write(dir.join("ignore.txt"), mk_frame("oc_c")).unwrap();

    let count = sebas::replay::run(sebas::replay::ReplayArgs { dir: dir.clone() })
        .await
        .expect("replay run");
    assert_eq!(
        count, 2,
        "two well-formed frames dispatched, bad one skipped"
    );

    let missing = sebas::replay::run(sebas::replay::ReplayArgs {
        dir: dir.join("does-not-exist"),
    })
    .await;
    assert!(missing.is_err(), "missing dir must error");

    let _ = std::fs::remove_dir_all(&dir);
}
