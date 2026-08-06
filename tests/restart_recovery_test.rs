//! spec §3.3e restart-recovery integration: a state file written by a
//! previous daemon run must restore as Dormant mappings; the first inbound
//! text lazily respawns via `session/load` (falling back to `session/new`),
//! and a corrupt state file is quarantined instead of aborting startup.

use acp_claude::manager::SessionManager;
use feishu::events::{FeishuIn, SessionKey};
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/fake-claude")
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
        })
        .await;
    let Out::SpawnResume {
        key: k,
        session_id: old,
        prompt,
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
async fn restored_mapping_resume_rejected_by_cli_surfaces_error() {
    // Post-ACP (Phase 1, sebas-dk8.2): there is no capability negotiation or
    // transparent session/new fallback. `resume` is claude-native; if the CLI
    // rejects the id (session files gone), the spawn itself fails and the
    // dispatcher drops the placeholder (fail_spawn) instead of silently
    // starting a fresh session. Phase 3 (sebas-dk8.4) makes this graceful.
    let json = r#"{"oc_restart":{"session_id":"sess-old","last_active_unix":1}}"#;
    let map = SessionMap::restore_json(json).unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    let mgr = Arc::new(SessionManager::new(Duration::from_millis(800)));

    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "hi".into(),
            reply_to: None,
        })
        .await;
    let Out::SpawnResume {
        key: k,
        session_id: old,
        prompt,
    } = first_out(&mut out_rx).await
    else {
        panic!("expected SpawnResume")
    };
    assert_eq!(old, "sess-old");

    // --resume-fails: the fake exits(1) at startup, modelling claude's
    // "No conversation found" — the resume must surface an error.
    let res = sebas::run::acp_resume_and_activate(
        &mgr,
        &router,
        &k,
        &old,
        &prompt,
        fake().to_str().unwrap(),
        vec!["--resume-fails".into()],
        None,
    )
    .await;
    assert!(res.is_err(), "rejected resume must error, got ok: {res:?}");

    // The dispatcher's error arm drops the Spawning placeholder so the next
    // text starts clean (this is what run.rs's SpawnResume arm does on Err).
    router.fail_spawn(&k).await;
    assert!(
        map.get(&key()).await.is_none(),
        "placeholder dropped after failed resume"
    );
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
