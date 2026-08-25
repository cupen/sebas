//! Second-turn ContinueSession: FSM flips ✅→🚧 and a fresh
//! ContinueSession is forwarded to the bridge.
//!
//! The router's responsibility on the second text after the first turn
//! has settled at ✅:
//!   1. UpdateCard🚧 (FSM flip)
//!   2. React 🚧 (FSM flip)
//!   3. SendAcp ContinueSession (forward to bridge)
//!
//! The actual second-turn event stream is exercised by `full_e2e_test`;
//! the fake CLI serves multi-turn prompts in streaming mode by default.

use acp_claude::manager::SessionManager;
use acp_claude::session::AcpCommand;
use feishu::cards::CardConfig;
use feishu::events::{FeishuIn, SessionKey};
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

mod support;

const OVERALL: Duration = Duration::from_secs(15);

fn workspace_target() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug")
}

#[tokio::test]
async fn second_text_flips_fsm_and_forwards_continue() {
    // Post-ACP: the manager drives the new-dialect fake CLI directly.
    // Windows 下可执行文件带 .exe 后缀。
    let fake = workspace_target().join(format!("fake-claude{}", std::env::consts::EXE_SUFFIX));
    assert!(fake.exists());

    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new_with_config(map, CardConfig::default(), 256);
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(15)));

    let key = SessionKey {
        chat_id: "oc_continue".into(),
        thread_id: None,
    };
    router
        .dispatch(FeishuIn::Text {
            key: key.clone(),
            text: "first".into(),
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

    let work_dir = support::TestDir::new("continue_session", "work");
    let (session_id, _pending, _rx) = sebas::run::acp_spawn_and_activate(
        &mgr,
        &router,
        &key,
        &prompt,
        fake.to_str().unwrap(),
        vec!["--slow-ms".into(), "200".into()],
        Some(work_dir.path().to_string_lossy().into_owned()),
        None,
    )
    .await
    .expect("spawn bridge");
    router.seed_card(session_id.clone(), prompt.clone()).await;
    router
        .record_root_msg_id(session_id.clone(), "om_fake_cont".into())
        .await;
    sebas::run::spawn_acp_pump(_rx, router.clone(), session_id.clone());
    if let Err(e) = sebas::run::flush_pending_prompts(&mgr, &session_id, _pending).await {
        panic!("flush_pending_prompts failed: {e}");
    }

    // 1) Drain Out stream until first React ✅ so the FSM reaches ✅.
    let deadline = std::time::Instant::now() + OVERALL;
    let mut first_done = false;
    while std::time::Instant::now() < deadline && !first_done {
        let got = match tokio::time::timeout(Duration::from_millis(500), out_rx.recv()).await {
            Ok(Some(o)) => o,
            Ok(None) => panic!("out_rx closed early"),
            Err(_) => continue,
        };
        if let Out::React { emoji, .. } = got
            && emoji == router::card_state::phase::DONE
        {
            first_done = true;
        }
    }
    assert!(first_done, "first turn did not settle at React ✅");

    // 2) Send a follow-up text. Capture the *ordered* sequence of Out
    //    values the router produces for the FSM-flip + continue-forward
    //    path. We only consume up to SendAcp ContinueSession.
    router
        .dispatch(FeishuIn::Text {
            key: key.clone(),
            text: "second".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;

    let mut saw_react_working = false;
    let mut cmd_prompt: Option<String> = None;
    let deadline = std::time::Instant::now() + OVERALL;
    while cmd_prompt.is_none() && std::time::Instant::now() < deadline {
        let got = match tokio::time::timeout(Duration::from_millis(500), out_rx.recv()).await {
            Ok(Some(o)) => o,
            Ok(None) => panic!("out_rx closed before SendAcp"),
            Err(_) => continue,
        };
        match got {
            Out::React { emoji, .. } if emoji == router::card_state::phase::WORKING => {
                saw_react_working = true
            }
            Out::SendAcp {
                cmd: AcpCommand::ContinueSession { prompt, .. },
                ..
            } => cmd_prompt = Some(prompt),
            _ => {}
        }
    }

    assert!(
        saw_react_working,
        "router did not emit React🚧 on turn-2 dispatch"
    );
    assert_eq!(
        cmd_prompt.as_deref(),
        Some("second"),
        "router did not forward ContinueSession with the new prompt"
    );

    mgr.kill_all().await;
    drop(mgr);
}
