//! ACP driver resume integration tests (openspec/changes/add-opencode-acp,
//! new `acp-driver` requirement "ACP session resume through session/load").
//!
//! These drive the *generic ACP driver* (`AcpDriver`, the `driver="acp"`
//! registry entry) against the programmable `fake-acp-agent` mock — NOT the
//! dedicated `fake-claude-cli`. The mock speaks real ACP v1 over stdio and
//! is scripted per scenario:
//! - `load-ok`     → advertises load_session, `session/load` succeeds
//! - `load-fails`  → advertises load_session, `session/load` errors
//! - `unavailable` → does not advertise load_session
//! - `--hang-init` → never answers `initialize` (handshake timeout)

use sebas_acp::claude::manager::SessionManager;
use sebas_acp::session::AcpEvent;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn fake_acp() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-acp-agent")
}

/// Build a manager whose sole registered agent is the generic ACP driver
/// bound to the mock binary with the given startup timeout.
fn acp_manager(startup_timeout: Duration) -> SessionManager {
    let mut agents = HashMap::new();
    agents.insert(
        "acp".to_string(),
        sebas_acp::claude::manager::AgentEntry {
            driver: Arc::new(sebas_acp::AcpDriver),
            startup_timeout,
        },
    );
    SessionManager::new("acp".to_string(), agents)
}

/// Helper: the mock binary path plus scenario arg.
fn mock_args(scenario: &str) -> Vec<String> {
    vec![fake_acp().to_string_lossy().into_owned(), scenario.into()]
}

/// Drain events until a TextDelta whose content contains `needle` arrives.
async fn wait_for_text(mgr: &SessionManager, id: &str, needle: &str) -> String {
    for _ in 0..20 {
        let evt = tokio::time::timeout(Duration::from_secs(3), mgr.next_event(id))
            .await
            .expect("timeout waiting for event")
            .expect("event stream closed");
        if let AcpEvent::TextDelta { delta, .. } = evt
            && delta.contains(needle)
        {
            return delta;
        }
    }
    panic!("no TextDelta containing {needle:?} within 20 events");
}

// ---------------------------------------------------------------------------
// resume success: load_session advertised, session/load succeeds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acp_resume_succeeds_keeps_id_and_resumed_true() {
    let mgr = acp_manager(Duration::from_secs(10));
    let outcome = mgr
        .resume_session("acp", mock_args("load-ok"), None, vec![], "sess-old-1", None)
        .await
        .expect("resume must succeed (load ok)");
    assert!(outcome.resumed, "load ok → resumed=true");
    assert_eq!(outcome.session_id, "sess-old-1", "routing id preserved on resume");

    // The loaded session answers prompts under the preserved id.
    mgr.send(
        &outcome.session_id,
        sebas_acp::session::AcpCommand::ContinueSession {
            session_id: outcome.session_id.clone(),
            prompt: "hi".into(),
        },
    )
    .await
    .expect("send prompt");

    // The mock echoes the ACP session id, which for a load is the
    // conversation id, and text deltas are stamped with the routing id.
    let text = wait_for_text(&mgr, &outcome.session_id, "echo:").await;
    assert!(
        text.contains("sess-old-1"),
        "prompt routed to the loaded conversation: {text}"
    );

    mgr.kill(&outcome.session_id).await;
}

// ---------------------------------------------------------------------------
// resume rejected: session gone → fresh fallback, new id, resumed=false
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acp_resume_load_failed_falls_back_to_fresh() {
    let mgr = acp_manager(Duration::from_secs(10));
    let outcome = mgr
        .resume_session(
            "acp",
            mock_args("load-fails"),
            None,
            vec![],
            "sess-deleted",
            None,
        )
        .await
        .expect("load failure must fall back, not error");
    assert!(!outcome.resumed, "fallback reports resumed=false");
    assert_ne!(
        outcome.session_id, "sess-deleted",
        "fallback mints a fresh routing id"
    );

    // The fresh session is fully functional under the new id.
    mgr.send(
        &outcome.session_id,
        sebas_acp::session::AcpCommand::ContinueSession {
            session_id: outcome.session_id.clone(),
            prompt: "hi".into(),
        },
    )
    .await
    .expect("send prompt");
    let text = wait_for_text(&mgr, &outcome.session_id, "echo:").await;
    assert!(
        text.contains("acp-new-"),
        "fresh prompt must land on the new ACP session, got: {text}"
    );

    mgr.kill(&outcome.session_id).await;
}

// ---------------------------------------------------------------------------
// resume when agent has no load capability → fresh fallback
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acp_resume_without_load_capability_falls_back_to_fresh() {
    let mgr = acp_manager(Duration::from_secs(10));
    let outcome = mgr
        .resume_session(
            "acp",
            mock_args("unavailable"),
            None,
            vec![],
            "sess-gone",
            None,
        )
        .await
        .expect("no load capability must fall back, not error");
    assert!(!outcome.resumed, "unavailable → resumed=false");
    assert_ne!(
        outcome.session_id, "sess-gone",
        "fallback mints a fresh routing id"
    );

    mgr.send(
        &outcome.session_id,
        sebas_acp::session::AcpCommand::ContinueSession {
            session_id: outcome.session_id.clone(),
            prompt: "hi".into(),
        },
    )
    .await
    .expect("send prompt");
    let text = wait_for_text(&mgr, &outcome.session_id, "echo:").await;
    assert!(text.contains("acp-new-"), "fresh prompt lands on new session: {text}");

    mgr.kill(&outcome.session_id).await;
}

// ---------------------------------------------------------------------------
// fresh spawn (no resume) still works: routing id minted, resumed=false
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acp_fresh_spawn_mints_id_and_reports_not_resumed() {
    let mgr = acp_manager(Duration::from_secs(10));
    let outcome = mgr
        .create_session("acp", mock_args("load-ok"), None, vec![], "".into())
        .await
        .expect("fresh spawn must succeed");
    assert!(!outcome.is_empty(), "fresh spawn mints a routing id");

    mgr.send(
        &outcome,
        sebas_acp::session::AcpCommand::CreateSession {
            session_id: outcome.clone(),
            prompt: "hi".into(),
        },
    )
    .await
    .expect("send prompt");
    let text = wait_for_text(&mgr, &outcome, "echo:").await;
    assert!(text.contains("acp-new-"), "fresh prompt lands on new session: {text}");

    mgr.kill(&outcome).await;
}

/// P3: `kill()` must hard-abort the driver run loop so the child process is
/// actually reaped (a cancel signal alone does not fire while the driver is
/// blocked awaiting a slow agent reply — the child would linger). We find the
/// mock's pid via /proc before killing and assert it is gone afterwards.
/// Liveness probing below is /proc-based → unix only (Windows liveness needs
/// a different mechanism; the reap semantics themselves are also unix).
#[cfg(unix)]
#[tokio::test]
async fn kill_reaps_child_process() {
    let mgr = acp_manager(Duration::from_secs(10));
    // A unique journal path doubles as a /proc marker for THIS test's child
    // (concurrent tests spawn their own mock agents).
    let journal_dir = tempfile::tempdir().expect("tempdir for journal");
    let journal = journal_dir.path().join("kill-journal.jsonl");
    let journal_str = journal.to_string_lossy().into_owned();
    let mut args = mock_args("load-ok");
    args.push("--journal".into());
    args.push(journal_str.clone());

    let outcome = mgr
        .create_session("acp", args, None, vec![], "".into())
        .await
        .expect("fresh spawn must succeed");

    // The mock agent child is alive right after spawn; poll briefly for the
    // pid (the driver spawns the subprocess inside its run task).
    let mut pid = None;
    for _ in 0..50 {
        if let Some(p) = find_fake_acp_pid(&journal_str) {
            pid = Some(p);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let pid = pid.expect("mock child must be running after spawn");

    mgr.kill(&outcome).await;
    // Aborting the run loop drops the connection → process-group kill.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !pid_alive(pid),
        "mock child pid {pid} must be gone after kill"
    );
}

/// Scan /proc for a live `fake-acp-agent` child whose cmdline carries the
/// unique journal path marker (so concurrent tests' children are ignored).
/// `/proc` root mixes PID dirs with pseudo files (`cpuinfo`, …) and entries
/// can vanish mid-scan, so non-PID/unreadable entries are skipped, not fatal.
#[cfg(unix)]
fn find_fake_acp_pid(journal_marker: &str) -> Option<u32> {
    for entry in std::fs::read_dir("/proc").ok()? {
        let Ok(entry) = entry else { continue };
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(cmdline) = std::fs::read_to_string(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        if cmdline.contains(journal_marker) {
            return Some(pid);
        }
    }
    None
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

// ---------------------------------------------------------------------------
// resume uses the caller-provided real ACP session id, not the routing id
// (openspec/changes/add-acp-session-id-mapping; the mock journals the id it
// was asked to load)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acp_resume_loads_by_provided_acp_session_id() {
    let mgr = acp_manager(Duration::from_secs(10));
    let journal_dir = tempfile::tempdir().expect("tempdir for journal");
    let journal = journal_dir.path().join("journal.jsonl");

    let outcome = mgr
        .resume_session(
            "acp",
            {
                let mut args = mock_args("load-ok");
                args.push("--journal".into());
                args.push(journal.to_string_lossy().into_owned());
                args
            },
            None,
            vec![],
            "sess-route-uuid",
            Some("acp-ses-real-42".to_string()),
        )
        .await
        .expect("resume must succeed (load ok)");
    assert!(outcome.resumed, "load ok → resumed=true");
    assert_eq!(
        outcome.session_id, "sess-route-uuid",
        "routing id preserved on resume"
    );
    assert_eq!(
        outcome.acp_session_id.as_deref(),
        Some("acp-ses-real-42"),
        "spawn outcome carries the ACP session id the load used"
    );
    // The in-manager record matches, so a future resume can re-issue the load.
    assert_eq!(
        mgr.get_acp_session_id(&outcome.session_id).await.as_deref(),
        Some("acp-ses-real-42"),
        "manager records the ACP session id for the routing id"
    );

    // The mock journaled every session/load request: assert the id it was
    // actually asked to load is the REAL ACP session id, not the routing id.
    let log = std::fs::read_to_string(&journal).expect("journal readable");
    let load_line = log
        .lines()
        .find(|l| l.contains("\"method\":\"session/load\""))
        .unwrap_or_else(|| panic!("no session/load in journal:\n{log}"));
    let parsed: serde_json::Value = serde_json::from_str(load_line).unwrap();
    assert_eq!(
        parsed["load_id"].as_str(),
        Some("acp-ses-real-42"),
        "driver must load by the provided ACP session id, not the routing id — journal: {log}"
    );

    // The loaded session answers prompts under the preserved routing id.
    mgr.send(
        &outcome.session_id,
        sebas_acp::session::AcpCommand::ContinueSession {
            session_id: outcome.session_id.clone(),
            prompt: "hi".into(),
        },
    )
    .await
    .expect("send prompt");
    let text = wait_for_text(&mgr, &outcome.session_id, "echo:").await;
    assert!(
        text.contains("acp-ses-real-42"),
        "prompt must be addressed to the loaded ACP session (its echo carries the ACP session id), got: {text}"
    );

    mgr.kill(&outcome.session_id).await;
}

// ---------------------------------------------------------------------------
// fresh spawn reports the agent's real ACP session id (session/new's id)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acp_fresh_spawn_reports_acp_session_id() {
    let mgr = acp_manager(Duration::from_secs(10));
    let sid = mgr
        .create_session("acp", mock_args("load-ok"), None, vec![], "".into())
        .await
        .expect("fresh spawn must succeed");
    // Fresh path reports the `session/new` id; the manager records it under
    // the routing id so a future resume loads by it (acp-session-mapping 场景 1).
    assert!(
        mgr.get_acp_session_id(&sid)
            .await
            .as_deref()
            .is_some_and(|id| id.starts_with("acp-new-")),
        "fresh spawn records the session/new id under the routing id, got {:?}",
        mgr.get_acp_session_id(&sid).await,
    );

    mgr.kill(&sid).await;
}

// ---------------------------------------------------------------------------
// handshake timeout: agent never answers initialize → spawn times out
// ---------------------------------------------------------------------------

#[tokio::test]
async fn acp_hanging_agent_times_out() {
    let mgr = acp_manager(Duration::from_millis(400));
    let mut args = mock_args("load-ok");
    args.push("--hang-init".into());
    let t0 = std::time::Instant::now();
    let res = mgr.create_session("acp", args, None, vec![], "".into()).await;
    let elapsed = t0.elapsed();
    let err = res.expect_err("hanging agent must not yield a session");
    assert!(
        err.to_string().contains("timed out"),
        "error should mention timeout: {err}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "timeout took too long: {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// model selection surface (add-acp-model-selection 1.1): the agent's
// `configOptions` model select is parsed into `SpawnOutcome.model`; an agent
// without a model option reports `model: None`.
// ---------------------------------------------------------------------------

/// `mock_args` plus `--model-options`（模型列表 + currentValue = 首项）。
fn mock_args_with_models(scenario: &str, models: &str) -> Vec<String> {
    let mut args = mock_args(scenario);
    args.push("--model-options".into());
    args.push(models.into());
    args
}

#[tokio::test]
async fn spawn_outcome_carries_model_info_from_config_options() {
    let mgr = acp_manager(Duration::from_secs(10));
    let sid = mgr
        .create_session(
            "acp",
            mock_args_with_models("load-ok", "free-model,pro-model,gemini-2.5"),
            None,
            vec![],
            "".into(),
        )
        .await
        .expect("fresh spawn with model options must succeed");
    let model = mgr.get_model_info(&sid).await.expect("agent exposed a model option");
    assert_eq!(model.current, "free-model");
    assert_eq!(model.options, vec!["free-model", "pro-model", "gemini-2.5"]);
    mgr.kill(&sid).await;
}

#[tokio::test]
async fn spawn_without_model_option_reports_none() {
    let mgr = acp_manager(Duration::from_secs(10));
    let sid = mgr
        .create_session("acp", mock_args("load-ok"), None, vec![], "".into())
        .await
        .expect("fresh spawn must succeed");
    assert_eq!(
        mgr.get_model_info(&sid).await,
        None,
        "agent without configOptions must report no model surface"
    );
    mgr.kill(&sid).await;
}

// ---------------------------------------------------------------------------
// SetModel (add-acp-model-selection 1.2/1.3): driver sends
// `session/set_config_option {configId:"model", value:<id>}` addressed to the
// REAL ACP session id (journaled by the mock); success → ModelChanged with the
// new current value; agent rejection → non-terminal Error and current unchanged.
// ---------------------------------------------------------------------------

/// Drain events until a `ModelChanged` or non-terminal `Error` arrives.
async fn wait_for_model_event(mgr: &SessionManager, id: &str) -> AcpEvent {
    for _ in 0..20 {
        let evt = tokio::time::timeout(Duration::from_secs(3), mgr.next_event(id))
            .await
            .expect("timeout waiting for model event")
            .expect("event stream closed");
        match &evt {
            AcpEvent::ModelChanged { .. } | AcpEvent::Error { terminal: false, .. } => return evt,
            AcpEvent::Error { terminal: true, .. } => {
                panic!("unexpected terminal error before model event: {evt:?}")
            }
            _ => {}
        }
    }
    panic!("no ModelChanged/Error within 20 events");
}

#[tokio::test]
async fn set_model_succeeds_and_emits_model_changed() {
    let mgr = acp_manager(Duration::from_secs(10));
    let journal_dir = tempfile::tempdir().expect("tempdir for journal");
    let journal = journal_dir.path().join("journal.jsonl");

    let mut args = mock_args_with_models("load-ok", "free-model,pro-model,gemini-2.5");
    args.push("--journal".into());
    args.push(journal.to_string_lossy().into_owned());
    let sid = mgr
        .create_session("acp", args, None, vec![], "".into())
        .await
        .expect("fresh spawn must succeed");
    assert_eq!(
        mgr.get_model_info(&sid).await.map(|m| m.current).as_deref(),
        Some("free-model"),
        "spawn outcome carries the agent's current model"
    );

    mgr.set_model(&sid, "pro-model")
        .await
        .expect("set model on a mong-model agent");
    let evt = wait_for_model_event(&mgr, &sid).await;
    match evt {
        AcpEvent::ModelChanged { model_id, .. } => {
            assert_eq!(model_id, "pro-model", "ModelChanged carries the accepted model");
        }
        other => panic!("expected ModelChanged, got {other:?}"),
    }

    // Wire-level: the mock journaled `session/set_config_option` with the REAL
    // ACP session id (not the routing uuid), configId "model", value "pro-model".
    let log = std::fs::read_to_string(&journal).expect("journal readable");
    let line = log
        .lines()
        .find(|l| l.contains("\"method\":\"session/set_config_option\""))
        .unwrap_or_else(|| panic!("no session/set_config_option in journal:\n{log}"));
    let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
    assert_eq!(parsed["config_id"].as_str(), Some("model"));
    assert_eq!(parsed["value"].as_str(), Some("pro-model"));
    assert_eq!(
        parsed["session_id"].as_str(),
        mgr.get_acp_session_id(&sid).await.as_deref(),
        "set_config_option must address the real ACP session id (mapping change 联动)"
    );

    mgr.kill(&sid).await;
}

#[tokio::test]
async fn set_model_rejected_leaves_current_unchanged() {
    let mgr = acp_manager(Duration::from_secs(10));
    let journal_dir = tempfile::tempdir().expect("tempdir for journal");
    let journal = journal_dir.path().join("journal.jsonl");

    let mut args = mock_args_with_models("load-ok", "free-model,pro-model");
    args.push("--reject-model".into());
    args.push("not-a-real-model".into());
    args.push("--journal".into());
    args.push(journal.to_string_lossy().into_owned());
    let sid = mgr
        .create_session("acp", args, None, vec![], "".into())
        .await
        .expect("fresh spawn must succeed");

    // 无效模型：agent 拒绝 → 显式错误、current 不变。
    mgr.set_model(&sid, "not-a-real-model")
        .await
        .expect("send-side must accept the command (the agent rejects it)");
    let evt = wait_for_model_event(&mgr, &sid).await;
    match evt {
        AcpEvent::Error {
            message,
            terminal: false,
            ..
        } => {
            assert!(
                message.contains("not-a-real-model"),
                "error names the rejected model: {message}"
            );
        }
        other => panic!("expected non-terminal Error for rejected model, got {other:?}"),
    }

    // current 不变（仍是 first option）；wire 上确实发出过 set_config_option。
    assert_eq!(
        mgr.get_model_info(&sid).await.map(|m| m.current).as_deref(),
        Some("free-model"),
        "rejected model must NOT change the session's current model"
    );
    let log = std::fs::read_to_string(&journal).expect("journal readable");
    assert!(
        log.contains("\"method\":\"session/set_config_option\""),
        "wire request must have been issued even for a rejected model:\n{log}"
    );

    mgr.kill(&sid).await;
}