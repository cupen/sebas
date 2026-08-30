//! Group-3 integration: with a slow session/new (fake --delay-new-ms), two
//! rapid texts produce exactly ONE session/new and TWO session/prompt calls
//! (the second being the drained pending queue, joined).

use sebas_acp_claude::manager::SessionManager;
use sebas_acp_claude::session::{AcpCommand, AcpEvent};
use sebas_feishu::events::{FeishuIn, SessionKey};
use sebas_router::router::{Out, RouterHandle};
use sebas_router::state::SessionMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/debug")
        .join(format!("fake-claude{}", std::env::consts::EXE_SUFFIX))
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
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;
    let out = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Out::SpawnAcp { key: k, prompt, .. } = out else {
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
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;

    let (session_id, pending, rx) = spawn.await.unwrap().expect("spawn ok");
    assert_eq!(pending, vec!["msg2".to_string()]);
    sebas::run::flush_pending_prompts(&mgr, &session_id, pending)
        .await
        .expect("flush");

    // Drain the session's events so the child finishes both turns.
    // `rx` was returned by acp_spawn_and_activate (cloned before the prompt
    // was sent); entry is present and session alive in this test.
    let mut finished = 0;
    let guard = tokio::time::timeout(Duration::from_secs(5), async {
        let mut rx = rx.lock().await;
        while finished < 2 {
            match rx.recv().await {
                Some(sebas_acp_claude::session::AcpEvent::Finished { .. }) => finished += 1,
                Some(_) => {}
                None => break,
            }
        }
    })
    .await;
    assert!(guard.is_ok() && finished == 2, "both turns should complete");

    // Journal (new dialect): exactly one initialize; two user messages
    // (initial prompt + flushed queue); second prompt carries msg2.
    let raw = std::fs::read_to_string(&journal).expect("journal");
    let news = raw.matches("\"subtype\": \"initialize\"").count()
        + raw.matches("\"subtype\":\"initialize\"").count();
    assert_eq!(news, 1, "expected exactly one initialize: {raw}");
    let prompts =
        raw.matches("\"type\": \"user\"").count() + raw.matches("\"type\":\"user\"").count();
    assert_eq!(prompts, 2, "expected two user messages: {raw}");
    assert!(
        raw.contains("msg2"),
        "pending prompt must reach the agent: {raw}"
    );
    let _ = std::fs::remove_file(&journal);
}

/// D6 regression: if the agent crashes on the first prompt, the terminal
/// `Error{terminal:true}` event must still reach the pump even though the
/// production path does `acp_spawn_and_activate` → `send_card` (a slow HTTP
/// round trip) → `spawn_acp_pump`. The fix clones the event receiver inside
/// `acp_spawn_and_activate` BEFORE the prompt is sent, so it survives the
/// wrapper's eager table removal when the process dies. Pre-fix, the pump
/// looked up `event_rx` AFTER the send_card delay — by which time the crash
/// had already removed the only Receiver Arc, losing the terminal event.
#[tokio::test]
async fn crash_on_first_prompt_reaches_pump_despite_slow_sendcard() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(30)));
    let key = SessionKey {
        chat_id: "oc_crash".into(),
        thread_id: None,
    };

    // Dispatch a text with "crash" -> SpawnAcp. The agent (fake-claude)
    // will send one chunk ("boom") then exit(2) — crashing during the first
    // turn, which is exactly D6's target scenario.
    router
        .dispatch(FeishuIn::Text {
            key: key.clone(),
            text: "crash".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;
    let out = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Out::SpawnAcp { key: k, prompt, .. } = out else {
        panic!("expected SpawnAcp, got {out:?}")
    };

    // Spawn the session with "crash" as the INITIAL prompt. The returned
    // `rx` is cloned before the prompt reaches the agent.
    let (session_id, _pending, rx) = sebas::run::acp_spawn_and_activate(
        &mgr,
        &router,
        &k,
        &prompt,
        fake().to_str().unwrap(),
        vec![],
        None,
        None,
    )
    .await
    .expect("spawn ok");

    // Simulate the production send_card delay (50-500ms HTTP round trip)
    // during which the agent crashes and the wrapper removes the table
    // entry — the ONLY Receiver Arc clone in the pre-fix code path.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Drain rx: the terminal Error event must survive because our rx clone
    // holds the receiver alive despite the table removal.
    let mut terminal_errors = 0;
    let mut stream_closed = false;
    let guard = tokio::time::timeout(Duration::from_secs(5), async {
        let mut rx = rx.lock().await;
        loop {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Some(evt)) => {
                    if matches!(evt, AcpEvent::Error { terminal: true, .. }) {
                        terminal_errors += 1;
                    }
                }
                Ok(None) => {
                    stream_closed = true;
                    break;
                }
                Err(_) => break, // per-event timeout
            }
        }
    })
    .await;
    assert!(guard.is_ok(), "drain should complete within 5s");
    assert_eq!(
        terminal_errors, 1,
        "exactly one terminal Error event must survive the table removal"
    );
    assert!(stream_closed, "stream must close after the terminal event");

    // The table entry was removed by the wrapper, so send must fail fast.
    let send_result = mgr
        .send(
            &session_id,
            AcpCommand::ContinueSession {
                session_id: session_id.clone(),
                prompt: "after crash".into(),
            },
        )
        .await;
    assert!(send_result.is_err(), "send must fail after table removal");
}
