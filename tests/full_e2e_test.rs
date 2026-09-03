//! Full integration: feishu → router → SessionManager → (cc-agent-sdk) →
//! fake-claude (new dialect), exercised through the same production functions
//! `run()` uses for its spawn path (`acp_spawn_and_activate` +
//! `spawn_acp_pump` + `flush_pending_prompts`). This skips the Feishu
//! HTTP/WS transport (no `FeishuClient`, no `dispatch_out`): `FeishuIn`
//! is injected straight into `RouterHandle::dispatch`, and `Out` events
//! are observed on the outbound channel — they ARE the prod intent,
//! they just don't traverse HTTP here.

use sebas_acp::claude::manager::SessionManager;
use sebas_feishu::cards::CardConfig;
use sebas_feishu::events::{FeishuIn, SessionKey};
use sebas_router::router::{Out, RouterHandle};
use sebas_router::state::SessionMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

mod support;

const OVERALL: Duration = Duration::from_secs(8);

fn workspace_target() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug")
}

#[tokio::test]
async fn dispatch_text_drives_bridge_to_finished_emoji() {
    let fake = workspace_target().join(format!("fake-claude{}", std::env::consts::EXE_SUFFIX));
    assert!(fake.exists(), "missing build artifact {}", fake.display());

    // Build the same shape `run()` constructs (run.rs:43–49).
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new_with_config(map, CardConfig::default(), 256);
    let mgr = Arc::new(SessionManager::claude_only(Duration::from_secs(15)));

    // 1) Inject a Feishu text. Router dispatch goes on_text → spawn_new →
    //    emit Out::SpawnAcp on the outbound channel.
    let key = SessionKey {
        chat_id: "oc_full_e2e".into(),
        thread_id: None,
    };
    router
        .dispatch(FeishuIn::Text {
            key: key.clone(),
            text: "hello".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;

    let spawn = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
        .await
        .expect("SpawnAcp not received in time")
        .expect("channel closed");
    let prompt = match spawn {
        Out::SpawnAcp { key: _, prompt, .. } => prompt,
        other => panic!("expected SpawnAcp, got {other:?}"),
    };
    assert_eq!(prompt, "hello");

    // 2) Run `acp_spawn_and_activate` (production fn): creates the bridge
    //    subprocess, completes the initialize handshake, sends the initial
    //    prompt, and returns the cloned event receiver taken before any
    //    slow I/O (D6 guarantee).
    let work_dir_a = support::TestDir::new("full_e2e", "first");
    let (session_id, pending, rx) = sebas::run::acp_spawn_and_activate(
        &mgr,
        &router,
        &key,
        &prompt,
        "claude",
        vec![fake.to_str().unwrap().to_string(), ],
        Some(work_dir_a.path().to_string_lossy().into_owned()),
        None,
    )
    .await
    .expect("spawn fake CLI through production fn");

    // 3) Whatever `wire_session_card_and_pump` does that's NOT a Feishu
    //    HTTP call: seed the card state, fake a recorded message_id (the
    //    real fn gets it from feishu.send_card; here we just need the
    //    router to know which message future React/UpdateCards should
    //    target if anyone wants them — which our test doesn't).
    router.seed_card(session_id.clone(), prompt.clone()).await;
    router
        .record_root_msg_id(session_id.clone(), "om_fake_e2e".into())
        .await;

    // 4) Production event pump: drains AcpEvent for the session,
    //    flushes UpdateCard on debounce, fires React on FSM transitions.
    sebas::run::spawn_acp_pump(rx.clone(), router.clone(), session_id.clone());

    // 5) Drain any prompts that arrived during the spawn window (none in
    //    this test, but the production fn always calls this — keeps the
    //    shape identical).
    if let Err(e) = sebas::run::flush_pending_prompts(&mgr, &session_id, pending).await {
        panic!("flush_pending_prompts failed: {e}");
    }

    // 6) Observe Out events on the same channel `dispatch_out` consumes
    //    in production. With the production spawn_acp_pump the 150ms
    //    debounce may coalesce the 🚧 flush into the terminal ✅ (the
    //    Finished event takes the immediate path before the first tick);
    //    the design intentionally drops the transient 🚧 in that
    //    race. We assert what the pump actually emits in this scenario:
    //    at least one UpdateCard and the terminal React ✅.
    //    (Lower-level tests without the debouncer cover the full
    //    👀→🚧→✅ FSM sequence.)
    let mut saw_update = false;
    let mut saw_react_done = false;
    let deadline = std::time::Instant::now() + OVERALL;
    while std::time::Instant::now() < deadline {
        let got = match tokio::time::timeout(Duration::from_millis(500), out_rx.recv()).await {
            Ok(Some(o)) => o,
            Ok(None) => panic!("out_rx closed before terminal emoji"),
            Err(_) => continue,
        };
        match got {
            Out::UpdateCard { .. } => saw_update = true,
            Out::React { emoji, .. } if emoji == sebas_router::card_state::phase::DONE => {
                saw_react_done = true;
                break;
            }
            _ => {}
        }
    }

    // Stop the session so spawn_acp_pump's recv() can return None and the
    // pump task can exit.
    mgr.kill_all().await;
    drop(mgr);

    assert!(saw_update, "no UpdateCard within {OVERALL:?}");
    assert!(saw_react_done, "no React ✅ within {OVERALL:?}");
}

/// Same flow as the fast test but the fake pauses between
/// the content frames and the result so the production pump's 150 ms debounce ticks
/// at least once before Finished arrives. That window is what exposes
/// the full 👀→🚧→✅ FSM on `out_rx`: the fast test can only show the
/// terminal ✅ because Finished takes the immediate path before any
/// `ticker.tick()`.
#[tokio::test]
async fn slow_stream_exposes_full_fsm_via_debounced_pump() {
    let fake = workspace_target().join(format!("fake-claude{}", std::env::consts::EXE_SUFFIX));
    assert!(fake.exists());

    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new_with_config(map, CardConfig::default(), 256);
    let mgr = Arc::new(SessionManager::claude_only(Duration::from_secs(15)));

    let key = SessionKey {
        chat_id: "oc_slow".into(),
        thread_id: None,
    };
    router
        .dispatch(FeishuIn::Text {
            key: key.clone(),
            text: "hello".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;
    let spawn = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
        .await
        .expect("SpawnAcp")
        .expect("closed");
    let prompt = match spawn {
        Out::SpawnAcp { prompt, .. } => prompt,
        other => panic!("expected SpawnAcp, got {other:?}"),
    };

    let work_dir_b = support::TestDir::new("full_e2e", "second");
    let (session_id, pending, rx) = sebas::run::acp_spawn_and_activate(
        &mgr,
        &router,
        &key,
        &prompt,
        "claude",
        // 800ms of mid-turn silence: pump startup (two process spawns) +
        // the 150ms debounce tick must both land before the result frame,
        // or the transient 🚧 flush is preempted by Finished.
        vec![fake.to_str().unwrap().to_string(), "--slow-ms".into(), "800".into()],
        Some(work_dir_b.path().to_string_lossy().into_owned()),
        None,
    )
    .await
    .expect("spawn fake CLI");
    router.seed_card(session_id.clone(), prompt.clone()).await;
    router
        .record_root_msg_id(session_id.clone(), "om_fake_slow".into())
        .await;
    sebas::run::spawn_acp_pump(rx, router.clone(), session_id.clone());
    if let Err(e) = sebas::run::flush_pending_prompts(&mgr, &session_id, pending).await {
        panic!("flush_pending_prompts failed: {e}");
    }

    // Walk out_rx in chronological order so we can confirm 🚧 precedes ✅.
    let mut saw_react_working = false;
    let mut saw_react_done = false;
    let mut working_before_done = false;
    let deadline = std::time::Instant::now() + OVERALL;
    while std::time::Instant::now() < deadline {
        let got = match tokio::time::timeout(Duration::from_millis(500), out_rx.recv()).await {
            Ok(Some(o)) => o,
            Ok(None) => panic!("out_rx closed early"),
            Err(_) => continue,
        };
        if let Out::React { emoji, .. } = &got {
            if emoji == sebas_router::card_state::phase::WORKING {
                saw_react_working = true;
            }
            if emoji == sebas_router::card_state::phase::DONE {
                saw_react_done = true;
                if saw_react_working {
                    working_before_done = true;
                }
                break;
            }
        }
    }

    mgr.kill_all().await;
    drop(mgr);

    assert!(saw_react_working, "no React 🚧 in slow stream");
    assert!(saw_react_done, "no React ✅ in slow stream");
    assert!(
        working_before_done,
        "🚧 reaction should arrive before ✅ (FSM order)"
    );
}
