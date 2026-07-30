//! Group-3 integration: with a slow session/new (fake --delay-new-ms), two
//! rapid texts produce exactly ONE session/new and TWO session/prompt calls
//! (the second being the drained pending queue, joined).

use feishu::events::{FeishuIn, SessionKey};
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use acp_claude::manager::SessionManager;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/fake-claude")
}

#[tokio::test]
async fn racing_texts_yield_one_spawn_and_joined_pending() {
    let journal = std::env::temp_dir().join(format!(
        "fc-journal-{}-race-{}.jsonl",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(30)));
    let key = SessionKey {
        chat_id: "oc_race".into(),
        thread_id: None,
    };

    // Text 1 -> SpawnAcp.
    router
        .dispatch(FeishuIn::Text {
            key: key.clone(),
            text: "msg1".into(),
            reply_to: None,
        })
        .await;
    let out = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Out::SpawnAcp { key: k, prompt } = out else {
        panic!("expected SpawnAcp, got {out:?}")
    };

    // Start the (slow) spawn in the background.
    let mgr2 = mgr.clone();
    let router2 = router.clone();
    let journal_arg = journal.to_str().unwrap().to_string();
    let spawn = tokio::spawn(async move {
        sebas::run::acp_spawn_and_activate(
            &mgr2,
            &router2,
            &k,
            &prompt,
            fake().to_str().unwrap(),
            vec![
                "--journal".into(),
                journal_arg,
                "--delay-new-ms".into(),
                "500".into(),
            ],
            None,
        )
        .await
    });

    // Text 2 arrives while session/new is sleeping -> queued, no second spawn.
    router
        .dispatch(FeishuIn::Text {
            key: key.clone(),
            text: "msg2".into(),
            reply_to: None,
        })
        .await;

    let (session_id, pending) = spawn.await.unwrap().expect("spawn ok");
    assert_eq!(pending, vec!["msg2".to_string()]);
    sebas::run::flush_pending_prompts(&mgr, &session_id, pending)
        .await
        .expect("flush");

    // Drain the session's events so the child finishes both turns.
    let rx = mgr.event_rx(&session_id).await.expect("event rx");
    let mut finished = 0;
    let guard = tokio::time::timeout(Duration::from_secs(5), async {
        let mut rx = rx.lock().await;
        while finished < 2 {
            match rx.recv().await {
                Some(acp_claude::session::AcpEvent::Finished { .. }) => finished += 1,
                Some(_) => {}
                None => break,
            }
        }
    })
    .await;
    assert!(guard.is_ok() && finished == 2, "both turns should complete");

    // Journal: exactly one session/new; two session/prompt; second prompt
    // carries msg2.
    let raw = std::fs::read_to_string(&journal).expect("journal");
    let news = raw.matches("\"session/new\"").count();
    assert_eq!(news, 1, "expected exactly one session/new: {raw}");
    let prompts = raw.matches("\"session/prompt\"").count();
    assert_eq!(prompts, 2, "expected two session/prompt calls: {raw}");
    assert!(
        raw.contains("msg2"),
        "pending prompt must reach the agent: {raw}"
    );
    let _ = std::fs::remove_file(&journal);
}
