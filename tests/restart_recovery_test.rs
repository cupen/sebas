//! restart-recovery integration (openspec/specs/acp-driver/spec.md): a state file written by a
//! previous daemon run must restore as Dormant mappings; the first inbound
//! text lazily respawns via claude-native `resume` (transparently falling
//! back to a fresh session when the conversation is gone — sebas-dk8.4),
//! and a corrupt state file is quarantined instead of aborting startup.

use acp_claude::manager::SessionManager;
use feishu::events::{FeishuIn, SessionKey};
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/debug")
        .join(format!("fake-claude{}", std::env::consts::EXE_SUFFIX))
}

fn key() -> SessionKey {
    SessionKey {
        chat_id: "oc_restart".into(),
        thread_id: None,
    }
}

async fn first_out(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Out {
    tokio::time::timeout(Duration::from_millis(500), rx.recv())
        .await
        .expect("out within 500ms")
        .expect("channel open")
}

#[tokio::test]
async fn restored_mapping_lazily_resumes_with_load_capable_agent() {
    // State file as a previous daemon's dump left it.
    let json = r#"{"oc_restart":{"session_id":"sess-old","last_active_unix":1}}"#;
    let map = SessionMap::restore_json(json).unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(30)));

    // First text after restart → SpawnResume (NOT a SendAcp black hole).
    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "继续上次".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;
    let Out::SpawnResume {
        key: k,
        session_id: old,
        prompt,
        ..
    } = first_out(&mut out_rx).await
    else {
        panic!("expected SpawnResume")
    };
    assert_eq!(old, "sess-old");
    assert_eq!(prompt, "继续上次");

    // The dispatcher arm: load-capable agent keeps the old id alive.
    let (sid, _pending, rx, resumed) = sebas::run::acp_resume_and_activate(
        &mgr,
        &router,
        &k,
        &old,
        &prompt,
        fake().to_str().unwrap(),
        vec![],
        None,
        None,
    )
    .await
    .expect("resume ok");
    assert!(resumed, "load-capable agent must resume the old session");
    assert_eq!(sid, "sess-old");
    // Mapping is Active again, keyed by the SAME id.
    assert_eq!(
        map.get(&key()).await.unwrap().session_id(),
        Some("sess-old")
    );

    // The triggering prompt flows: deltas then Finished.
    let mut got_finished = false;
    let guard = tokio::time::timeout(Duration::from_secs(5), async {
        let mut rx = rx.lock().await;
        while let Some(evt) = rx.recv().await {
            if matches!(evt, acp_claude::session::AcpEvent::Finished { .. }) {
                got_finished = true;
                break;
            }
        }
    })
    .await;
    assert!(guard.is_ok() && got_finished, "prompt turn should complete");

    mgr.kill(&sid).await;
}

#[tokio::test]
async fn restored_mapping_resume_rejected_falls_back_to_fresh() {
    // sebas-dk8.4: claude rejecting the resume (session files cleaned —
    // the fake exits(1) with "No conversation found") must NOT surface as a
    // spawn failure. The manager transparently starts a fresh session under
    // a NEW id and reports resumed=false; run.rs then sends the user a
    // session-lost notice (asserted at the card level in the e2e suite).
    let json = r#"{"oc_restart":{"session_id":"sess-old","last_active_unix":1}}"#;
    let map = SessionMap::restore_json(json).unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(30)));

    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "hi".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;
    let Out::SpawnResume {
        key: k,
        session_id: old,
        prompt,
        ..
    } = first_out(&mut out_rx).await
    else {
        panic!("expected SpawnResume")
    };
    assert_eq!(old, "sess-old");

    // The 15s guard proves the fallback fires via the stderr watch, not
    // by riding out the 30s startup timeout.
    let (sid, _pending, rx, resumed) = tokio::time::timeout(
        Duration::from_secs(15),
        sebas::run::acp_resume_and_activate(
            &mgr,
            &router,
            &k,
            &old,
            &prompt,
            fake().to_str().unwrap(),
            vec!["--resume-fails".into()],
            None,
            None,
        ),
    )
    .await
    .expect("fallback must be fast, not a startup-timeout hang")
    .expect("rejected resume falls back instead of erroring");

    assert!(!resumed, "fallback reports resumed=false");
    assert_ne!(sid, "sess-old", "fallback mints a fresh routing id");
    // The mapping activated under the FRESH id (no fail_spawn, no stale id).
    assert_eq!(
        map.get(&key()).await.unwrap().session_id(),
        Some(sid.as_str())
    );

    // The triggering prompt still completes its turn on the fresh session.
    let mut got_finished = false;
    let guard = tokio::time::timeout(Duration::from_secs(5), async {
        let mut rx = rx.lock().await;
        while let Some(evt) = rx.recv().await {
            if matches!(evt, acp_claude::session::AcpEvent::Finished { .. }) {
                got_finished = true;
                break;
            }
        }
    })
    .await;
    assert!(guard.is_ok() && got_finished, "prompt turn should complete");

    mgr.kill(&sid).await;
}

#[tokio::test]
async fn corrupt_state_file_is_quarantined_not_fatal() {
    let dir = std::env::temp_dir().join(format!(
        "sebas-corrupt-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let state = dir.join("sessions.json");
    std::fs::write(&state, "{ not json !!").unwrap();

    let map = sebas::run::restore_session_map(state.to_str().unwrap(), 8);
    // Boot succeeded with an empty table...
    assert!(map.get(&key()).await.is_none());
    // ...and the corrupt file was moved aside, not left to trip the next boot.
    assert!(!state.exists(), "corrupt file must be renamed away");
    let quarantined = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("sessions.json.corrupt-"));
    assert!(quarantined.is_some(), "quarantine file exists");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn missing_and_empty_state_files_boot_empty() {
    let dir = std::env::temp_dir().join(format!("sebas-boot-{}", std::process::id()));
    let missing = dir.join("nope.json");
    let map = sebas::run::restore_session_map(missing.to_str().unwrap(), 8);
    assert!(map.get(&key()).await.is_none());

    std::fs::create_dir_all(&dir).unwrap();
    let empty = dir.join("empty.json");
    std::fs::write(&empty, "").unwrap();
    let map = sebas::run::restore_session_map(empty.to_str().unwrap(), 8);
    assert!(map.get(&key()).await.is_none());
    // Empty file is NOT quarantined — it is a valid empty state.
    assert!(empty.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
