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

use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::AcpCommand;
use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_dispatch::cards::CardConfig;
use sebas_dispatch::engine::{DispatchHandle, Out};
use sebas_dispatch::state::SessionMap;
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
    let (router, mut out_rx) = DispatchHandle::new_with_config(map, CardConfig::default(), 256);
    let mgr = Arc::new(SessionManager::claude_only(Duration::from_secs(15)));

    let key = ChannelKey::feishu("oc_continue", None);
    router
        .dispatch(ChannelEvent::Text {
            key: key.clone(),
            text: "first".into(),
            reply_target: None,
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
    let (session_id, _pending, _rx, _model_info) = sebas::run::acp_spawn_and_activate(
        &mgr,
        &router,
        &key,
        &prompt,
        "claude",
        vec![
            fake.to_str().unwrap().to_string(),
            "--slow-ms".into(),
            "200".into(),
        ],
        Some(work_dir.path().to_string_lossy().into_owned()),
        None,
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
            && emoji == sebas_dispatch::card_state::phase::DONE
        {
            first_done = true;
        }
    }
    assert!(first_done, "first turn did not settle at React ✅");

    // 2) Send a follow-up text. Capture the *ordered* sequence of Out
    //    values the router produces for the FSM-flip + continue-forward
    //    path. We only consume up to SendAcp ContinueSession.
    router
        .dispatch(ChannelEvent::Text {
            key: key.clone(),
            text: "second".into(),
            reply_target: None,
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
            Out::React { emoji, .. } if emoji == sebas_dispatch::card_state::phase::WORKING => {
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

/// workbench-turn-queue 2.1：WORKING 时 web 与 feishu 两条提交路径都入队、
/// 都不发 SendAcp——in-flight 判定收敛在共享的 submit_turn（design D3）。
#[tokio::test]
async fn web_and_feishu_paths_both_enqueue_while_working_without_sendacp() {
    let map = SessionMap::new();
    let (router, mut out_rx) = DispatchHandle::new_with_config(map, CardConfig::default(), 256);
    let key = ChannelKey::new("web", "web-tq");
    router.insert_mapping(key.clone(), "s1".into()).await;
    router.seed_card("s1".into(), "first".into()).await;
    // SEED → WORKING：卡片进入在飞状态。
    router
        .dispatch_acp_event(sebas_acp::claude::session::AcpEvent::TextDelta {
            session_id: "s1".into(),
            delta: "streaming...".into(),
        })
        .await;

    // web 路径：忙中提交 → 入队。
    router
        .web_send_message(key.clone(), "from web".into())
        .await
        .expect("accepted (queued)");
    // feishu 路径：同一条映射上的入站文本 → 同一判定 → 入队。
    router
        .dispatch(ChannelEvent::Text {
            key: key.clone(),
            text: "from feishu".into(),
            reply_target: None,
        })
        .await;

    assert_eq!(
        router.map.queue_len(&key).await,
        2,
        "both submissions queued"
    );

    // 出站流里绝不出现这两条 prompt 的 SendAcp（只有 ⏳/FSM reaction 等）。
    let mut deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(100), out_rx.recv()).await {
            Ok(Some(Out::SendAcp {
                cmd: AcpCommand::ContinueSession { prompt, .. },
                ..
            })) => panic!("SendAcp must not fire while WORKING, got prompt {prompt:?}"),
            Ok(Some(_)) => continue,
            _ => deadline = std::time::Instant::now(),
        }
    }
}

/// workbench-turn-queue 2.2：入队后 transcript 无该 prompt；本轮结束后
/// drain 开轮，prompt 以条目出现在队尾（design D4——prompt 落点 = 开轮）。
#[tokio::test]
async fn queued_submission_enters_transcript_only_when_its_turn_starts() {
    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new_with_config(map, CardConfig::default(), 256);
    let key = ChannelKey::new("web", "web-tq2");
    router.insert_mapping(key.clone(), "s1".into()).await;
    router.seed_card("s1".into(), "first".into()).await;
    router
        .dispatch_acp_event(sebas_acp::claude::session::AcpEvent::TextDelta {
            session_id: "s1".into(),
            delta: "streaming...".into(),
        })
        .await;

    router
        .web_send_message(key.clone(), "queued text".into())
        .await
        .expect("accepted (queued)");

    // 入队后：transcript 没有这条 prompt（首条 prompt + 流式 delta 共 2 条）。
    let turns = router.session_turns(&key, 0).await.unwrap();
    assert_eq!(
        turns.len(),
        2,
        "submission time must not write a prompt entry"
    );
    assert!(!turns.iter().any(|e| e.content.contains("queued text")));

    // 本轮结束（Finished）→ drain 开轮：prompt 落在队尾。
    router
        .dispatch_acp_event(sebas_acp::claude::session::AcpEvent::Finished {
            session_id: "s1".into(),
        })
        .await;
    let turns = router.session_turns(&key, 0).await.unwrap();
    let last = turns.last().expect("drain seeded the prompt entry");
    assert_eq!(last.kind, "prompt");
    assert_eq!(
        last.content, "queued text",
        "prompt appears at the tail when its turn starts"
    );
}
