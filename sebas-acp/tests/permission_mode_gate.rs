//! permission-mode-auto-gate 集成测试：mode 门控的端到端行为。
//!
//! 驱动层的门控单测（hook 回调直读共享 mode 单元）在
//! `src/claude/driver.rs::tests`；本文件用 fake-claude 走完整 manager 链路：
//! - resume 携带 `--permission-mode` argv（dispatch 把映射 desired_mode 翻译
//!   成的启动值）：首个工具调用 hook 静默——零 PermissionRequest、零
//!   hook_callback，工具直接执行（spec 场景「resume re-issues the session
//!   mode」+「hook side gate is silent across surfaces」）。
//! - 运行时 SetMode(auto)：下一次工具调用零弹卡、无需 respawn（spec 场景
//!   「runtime switch takes effect on the next hook consult」）。

use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::{AcpCommand, AcpEvent};
use std::path::PathBuf;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude-cli")
}

/// 纳秒级唯一 journal 名，避免并发测试互写。
fn unique_journal(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "fc-journal-{}-{tag}-{}.jsonl",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ))
}

/// 等一个事件；PermissionRequest 直接视为失败（bypass 档必须静默）。
async fn next_event(mgr: &SessionManager, id: &str) -> AcpEvent {
    let evt = tokio::time::timeout(Duration::from_secs(3), mgr.next_event(id))
        .await
        .expect("event timeout")
        .expect("event stream open");
    assert!(
        !matches!(evt, AcpEvent::PermissionRequest { .. }),
        "bypass tier must not produce a PermissionRequest, got {evt:?}"
    );
    evt
}

/// resume 携带 `--permission-mode bypassPermissions`（desired_mode=auto 经
/// dispatch `resume_command_with_mode` 翻译出的 argv）：首个工具调用 hook
/// 静默——事件流零 PermissionRequest、journal 零 hook_callback，且 argv
/// 原样抵达子进程（driver 不吞不改写该 flag）。
#[tokio::test]
async fn resume_with_desired_mode_auto_keeps_first_tool_call_hook_silent() {
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    let journal = unique_journal("resume-mode");
    let outcome = mgr
        .resume_claude_session(
            fake().to_str().unwrap(),
            vec![
                "--permission-mode".into(),
                "bypassPermissions".into(),
                "--journal".into(),
                journal.to_str().unwrap().into(),
            ],
            None,
            vec![],
            "sess-mode-auto",
        )
        .await
        .expect("resume_session");
    assert!(outcome.resumed, "resume keeps the conversation id");

    // 首个工具调用：ask 档本必弹卡的 perm prompt 在 bypass 档静默直达收尾。
    mgr.send(
        &outcome.session_id,
        AcpCommand::ContinueSession {
            session_id: outcome.session_id.clone(),
            prompt: "perm".into(),
        },
    )
    .await
    .expect("send perm prompt");

    let mut saw_tool_end = false;
    let mut finished = false;
    for _ in 0..10 {
        match next_event(&mgr, &outcome.session_id).await {
            AcpEvent::ToolEnd {
                tool_name, result, ..
            } => {
                assert_eq!(tool_name, "Bash");
                assert!(result.contains("perm done"), "tool ran silently: {result}");
                saw_tool_end = true;
            }
            AcpEvent::Finished { .. } => {
                finished = true;
                break;
            }
            _ => {}
        }
    }
    assert!(
        saw_tool_end && finished,
        "turn must complete without any permission roundtrip"
    );

    // Journal：argv 原样携带 flag（翻译链的驱动半边），全程零 hook_callback。
    let raw = std::fs::read_to_string(&journal).expect("journal exists");
    let meta = raw
        .lines()
        .find(|l| l.contains("\"dir\":\"meta\""))
        .expect("argv meta line journaled");
    assert!(
        meta.contains("--permission-mode") && meta.contains("bypassPermissions"),
        "resume argv must re-issue --permission-mode: {meta}"
    );
    assert!(
        !raw.lines().any(|l| l.contains("hook_callback")),
        "hook must stay silent under bypass tier"
    );
    let _ = std::fs::remove_file(&journal);
    mgr.kill(&outcome.session_id).await;
}

/// 运行时 SetMode(auto) 即时生效：首个 perm 工具调用照常弹卡（ask 默认档），
/// SetMode 被接受（ModeChanged）后，下一次工具调用零弹卡——无 respawn
/// （journal 只有一段进程、一次 hook_callback + 一次 mode_change）。
#[tokio::test]
async fn set_mode_auto_silences_the_next_tool_call_without_respawn() {
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    let journal = unique_journal("setmode");
    let id = mgr
        .create_claude_session(
            fake().to_str().unwrap(),
            vec![
                "--journal".into(),
                journal.to_str().unwrap().into(),
            ],
            None,
            vec![],
            "".into(),
        )
        .await
        .expect("spawn");

    // 第一个 perm prompt（默认 ask 档）：照常产生 PermissionRequest。
    mgr.send(
        &id,
        AcpCommand::CreateSession {
            session_id: id.clone(),
            prompt: "perm".into(),
        },
    )
    .await
    .expect("prompt perm");
    let mut request_id = None;
    for _ in 0..10 {
        let evt = tokio::time::timeout(Duration::from_secs(3), mgr.next_event(&id))
            .await
            .expect("event timeout")
            .expect("stream open");
        if let AcpEvent::PermissionRequest { request_id: rid, .. } = evt {
            request_id = Some(rid);
            break;
        }
    }
    let request_id = request_id.expect("ask tier must park a permission request");

    // 泊车期间切 auto：SetMode 被执行体接受 → ModeChanged（控制面事实）。
    mgr.send(
        &id,
        AcpCommand::SetMode {
            session_id: id.clone(),
            mode: "auto".into(),
        },
    )
    .await
    .expect("send SetMode");
    let mut mode_changed = false;
    for _ in 0..10 {
        let evt = tokio::time::timeout(Duration::from_secs(3), mgr.next_event(&id))
            .await
            .expect("event timeout")
            .expect("stream open");
        match evt {
            AcpEvent::ModeChanged { mode, .. } => {
                assert_eq!(mode, "auto");
                mode_changed = true;
                break;
            }
            AcpEvent::Error { message, .. } => {
                panic!("SetMode(auto) must be accepted, got error: {message}");
            }
            _ => {}
        }
    }
    assert!(mode_changed, "no ModeChanged for SetMode(auto)");

    // 放行当前请求（首要语义不回退），回合收尾。
    mgr.send(
        &id,
        AcpCommand::PermissionReply {
            session_id: id.clone(),
            request_id,
            decision: sebas_acp::claude::session::Decision::AllowOnce,
        },
    )
    .await
    .expect("reply");
    for _ in 0..10 {
        if let AcpEvent::Finished { .. } = next_event(&mgr, &id).await {
            break;
        }
    }

    // 第二个 perm prompt：下一次 hook 咨询已读新档——零弹卡直达收尾。
    mgr.send(
        &id,
        AcpCommand::ContinueSession {
            session_id: id.clone(),
            prompt: "perm".into(),
        },
    )
    .await
    .expect("second perm prompt");
    let mut saw_tool_end = false;
    let mut finished = false;
    for _ in 0..10 {
        match next_event(&mgr, &id).await {
            AcpEvent::ToolEnd { result, .. } => {
                assert!(result.contains("perm done"), "tool ran silently: {result}");
                saw_tool_end = true;
            }
            AcpEvent::Finished { .. } => {
                finished = true;
                break;
            }
            _ => {}
        }
    }
    assert!(
        saw_tool_end && finished,
        "second gated call must complete without prompting"
    );

    // Journal：驱动确实下发了 set_permission_mode；两跳 hook_callback 都到
    // 达驱动，但第二跳的应答出自 mode 门控（reason 带 bypass tier 标记），
    // 而非用户点击；单一子进程（无 respawn）。注意 fake 对泊车中途的
    // set_permission_mode 只 ack 不改自身档位（main-loop handler 才更新），
    // 所以第二跳仍会咨询——正是这条 journal 证明静默出自**驱动门控**，
    // 而非 fake 自身的跳过逻辑。
    let raw = std::fs::read_to_string(&journal).expect("journal exists");
    assert_eq!(
        raw.lines()
            .filter(|l| l.contains("set_permission_mode") && l.contains("\"dir\":\"in\""))
            .count(),
        1,
        "driver must issue exactly one set_permission_mode: {raw}"
    );
    assert_eq!(
        raw.lines()
            .filter(|l| l.contains("hook_callback") && l.contains("control_request"))
            .count(),
        2,
        "both gated calls consult the hook (fake stays in ask tier): {raw}"
    );
    let last_hook_answer = raw
        .lines()
        .filter(|l| l.contains("control_response") && l.contains("permissionDecision"))
        .last()
        .expect("second consult must be answered")
        .to_string();
    assert!(
        last_hook_answer.contains("allowed by session permission mode (bypass tier)"),
        "second consult must be resolved by the mode gate, not a user click: {last_hook_answer}"
    );
    assert_eq!(
        raw.lines().filter(|l| l.contains("\"dir\":\"meta\"")).count(),
        1,
        "single child process — no respawn for the mode switch: {raw}"
    );
    let _ = std::fs::remove_file(&journal);
    mgr.kill(&id).await;
}
