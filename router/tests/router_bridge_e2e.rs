//! End-to-end: router ↔ in-tree acp-claude bridge ↔ fake-stream-claude
//!
//! 不依赖真实 Feishu。
//! 流程：router.dispatch(FeishuIn::Text) → SpawnAcp → 手动起 bridge
//! → SessionManager 接 → 灌 AcpEvent → router → out_rx 断言

use acp_claude::manager::SessionManager;
use acp_claude::session::AcpCommand;
use feishu::events::{FeishuIn, SessionKey};
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use std::sync::Arc;
use std::time::Duration;

const BRIDGE: &str = "/home/bot/workbench/repos/sebas/target/debug/claude-acp-bridge";
const FAKE_CLAUDE: &str = "/home/bot/workbench/repos/sebas/target/debug/fake-stream-claude";
const OVERALL: Duration = Duration::from_secs(8);

fn key() -> SessionKey {
    SessionKey {
        chat_id: "oc_test_chat".into(),
        thread_id: None,
    }
}

#[tokio::test]
async fn hello_text_drives_bridge_to_finished_emoji() {
    std::env::set_var("SEBAS_CLAUDE_PATH", FAKE_CLAUDE);

    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(15)));

    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "hello".into(),
            reply_to: None,
        })
        .await;

    let first = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
        .await
        .expect("SpawnAcp not received in time")
        .expect("channel closed");
    let prompt = match first {
        Out::SpawnAcp { key: _, prompt } => prompt,
        other => panic!("expected SpawnAcp, got {other:?}"),
    };
    assert_eq!(prompt, "hello");

    let session_id = mgr
        .create_session(BRIDGE, vec!["hello".into()], Some("/tmp".into()), "".into())
        .await
        .expect("spawn bridge");

    router.activate(&key(), session_id.clone()).await;

    let router_for_pump = router.clone();
    let session_id_for_pump = session_id.clone();
    let mgr_for_pump = Arc::clone(&mgr);
    let pump = tokio::spawn(async move {
        let rx = mgr_for_pump.event_rx(&session_id_for_pump).await.expect("event_rx");
        let mut rx_guard = rx.lock().await;
        while let Some(evt) = rx_guard.recv().await {
            router_for_pump.dispatch_acp_event(evt).await;
        }
    });

    mgr.send(
        &session_id,
        AcpCommand::CreateSession {
            session_id: session_id.clone(),
            prompt: "hello".into(),
        },
    )
    .await
    .expect("send prompt");

    let mut got_finished = false;
    let deadline = std::time::Instant::now() + OVERALL;
    while std::time::Instant::now() < deadline && !got_finished {
        match tokio::time::timeout(Duration::from_millis(500), out_rx.recv()).await {
            Ok(Some(Out::React { emoji, .. })) if emoji == router::card_state::phase::DONE => {
                got_finished = true;
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("out_rx closed"),
            Err(_) => continue,
        }
    }

    // 收到 ✅ 后主动 stop 掉 bridge 子进程，否则 evt_tx 不 drop，pump 的
    // recv() 永远阻塞 → 测试 hang。
    mgr.kill_all().await;

    drop(mgr);
    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;

    assert!(got_finished, "no React ✅ within {OVERALL:?}");
}
