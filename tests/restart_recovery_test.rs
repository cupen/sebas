//! restart-recovery integration (openspec/specs/session-lifecycle/spec.md):
//! the session map restored from the state store comes back as Dormant
//! mappings; the first inbound text lazily respawns via claude-native
//! `resume` (transparently falling back to a fresh session when the
//! conversation is gone — sebas-dk8.4). persist-session-map 4.1：恢复源是
//! 状态库（projects.db 的 session_map 表），不再是文件——
//!
//! - 映射**条目**不可读（不可寻址行）→ 启动为空表且不阻塞；
//! - 空库 → 空表启动，无错误；
//! - 库**本身**打不开 → 按状态库损坏规则拒启（绝不重置，绝不静默重建），
//!   会话映射恢复不参与该裁决。

use sebas_acp::claude::manager::SessionManager;
use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_dispatch::engine::{DispatchHandle, Out};
use sebas_dispatch::state::SessionMap;
use sebas_models::session_map::SessionMapRow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/debug")
        .join(format!("fake-claude{}", std::env::consts::EXE_SUFFIX))
}

fn key() -> ChannelKey {
    ChannelKey::feishu("oc_restart", None)
}

/// 一行「上次 daemon 留下的」持久化映射（feishu 会话无 project_dir 亦可）。
fn dormant_row(session_id: &str) -> SessionMapRow {
    SessionMapRow {
        chat_id: "feishu".into(),
        thread_id: Some("oc_restart".into()),
        session_id: session_id.into(),
        last_active_unix: 1,
        project_dir: None,
        acp_session_id: None,
        current_model: None,
        pending_kind: None,
        pending_model: None,
        pending_mode: None,
        desired_mode: sebas_dispatch::engine::ask_mode(),
        label: None,
        prompt_preview: None,
        awaiting_first_prompt: false,
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
    // State store rows as a previous daemon's per-mutation writes left them.
    let map = SessionMap::restore_rows(vec![dormant_row("sess-old")], usize::MAX);
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    let mgr = Arc::new(SessionManager::claude_only(Duration::from_secs(30)));

    // First text after restart → SpawnResume (NOT a SendAcp black hole).
    router
        .dispatch(ChannelEvent::Text {
            key: key(),
            text: "继续上次".into(),
            reply_target: None,
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
        "claude",
        vec![fake().to_str().unwrap().to_string()],
        None,
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
            if matches!(evt, sebas_acp::claude::session::AcpEvent::Finished { .. }) {
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
    let map = SessionMap::restore_rows(vec![dormant_row("sess-old")], usize::MAX);
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    let mgr = Arc::new(SessionManager::claude_only(Duration::from_secs(30)));

    router
        .dispatch(ChannelEvent::Text {
            key: key(),
            text: "hi".into(),
            reply_target: None,
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
            "claude",
            vec![
                fake().to_str().unwrap().to_string(),
                "--resume-fails".into(),
            ],
            None,
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
            if matches!(evt, sebas_acp::claude::session::AcpEvent::Finished { .. }) {
                got_finished = true;
                break;
            }
        }
    })
    .await;
    assert!(guard.is_ok() && got_finished, "prompt turn should complete");

    mgr.kill(&sid).await;
}

/// 「映射条目不可读」的新形态：行缺会话键（不可寻址）→ 恢复跳过该行，
/// 启动为空表、绝不阻塞（对应旧「损坏文件被隔离不致命」的覆盖点）。
#[tokio::test]
async fn unreadable_entries_boot_empty_without_blocking() {
    let mut unreadable = dormant_row("s-orphan");
    unreadable.thread_id = None;
    let map = SessionMap::restore_rows(vec![unreadable], 8);
    // Boot succeeded with an empty table — no panic, no rejection.
    assert!(map.get(&key()).await.is_none());
    // 可寻址的行照常恢复：一条坏行绝不拖累其余条目。
    let map = SessionMap::restore_rows(vec![dormant_row("sess-ok")], 8);
    assert_eq!(
        map.get(&key()).await.unwrap().transcript_id(),
        Some("sess-ok"),
        "good entries restore alongside skipped ones"
    );
}

/// 空库（无任何持久化映射）→ 空表启动，无错误（对应旧「缺失/空文件」
/// 覆盖点——文件不存在与否不再是概念，空表即首启）。
#[tokio::test]
async fn empty_store_boots_empty() {
    let map = SessionMap::restore_rows(Vec::new(), 8);
    assert!(map.get(&key()).await.is_none());
}

/// 库本身打不开（损坏）→ 按状态库损坏规则拒启：写者拒绝初始化、点名路径、
/// 文件原样不动；会话映射恢复绝不重置或重建库（session-lifecycle delta：
/// 「governed by the state store's corruption rule」）。
#[test]
fn corrupt_store_refuses_to_start_and_is_never_reset() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("projects.db");
    let garbage = b"not a sqlite database at all".to_vec();
    std::fs::write(&db, &garbage).unwrap();

    // 生产装配路径（run.rs 的 start_projects）如实拒绝。
    let err = sebas::sebas_state::writer::StateWriter::start_projects(db.clone())
        .err()
        .expect("corrupt projects.db must refuse to start");
    assert!(
        err.contains("初始化失败") || err.contains("损坏"),
        "diagnostic must name the corruption: {err}"
    );
    // 文件原样：绝不重置、绝不隔离、绝不重建。
    assert_eq!(
        std::fs::read(&db).unwrap(),
        garbage,
        "corrupt store must not be reset or recreated by recovery"
    );
    // 跨重启同样拒启（稳定性）。
    assert!(sebas::sebas_state::writer::StateWriter::start_projects(db).is_err());
}
