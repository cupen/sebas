//! workbench-composer-input-polish 2.2/2.3：claude 驱动的模型面。
//!
//! 覆盖（openspec/changes/workbench-composer-input-polish/specs/
//! acp-model-selection/spec.md 的 claude 场景）：
//! - 握手成功即拼装 `AcpModelInfo { current, options }` 上报（别名表内置
//!   default/opus/sonnet/haiku，current 缺省 "default"）；
//! - `[acp.agents.<name>] models` 覆盖值经 `ClaudeDriver::with_models` 替换
//!   内置表（进程级形态：config 解析单测在 tests/config_test.rs）；
//! - wire 帧观察（system init 帧 model 名）发 `ModelChanged`——快照的
//!   current 从 "default" 覆盖为观察值（D5 自愈语义）；
//! - `AcpCommand::SetModel` 走 SDK `set_model` 控制协议（"default" → None）：
//!   journal 记录控制请求、乐观 `ModelChanged`；切回 "default" 后观察值
//!   纠偏覆盖乐观值（fake 的 assistant 帧回到 "fake"）。
//!
//! 数据源：fake-claude（system init 帧 model "fake"、assistant 帧 model
//! 随 set_model 更新、journal 记 `model_change`）。

use sebas_acp::claude::manager::{AgentEntry, SessionManager};
use sebas_acp::claude::session::{AcpCommand, AcpEvent};
use sebas_acp::{ClaudeDriver, AcpModelInfo};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude-cli")
}

/// 每个测试独享的 journal 路径（并行测试互不串写）。
fn journal(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sebas-claude-model-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("journal dir");
    dir.join(format!("{tag}.jsonl"))
}

async fn drain_until<F: Fn(&AcpEvent) -> bool>(
    mgr: &SessionManager,
    id: &str,
    pred: F,
) -> AcpEvent {
    for _ in 0..20 {
        let evt = tokio::time::timeout(Duration::from_secs(5), mgr.next_event(id))
            .await
            .expect("event timeout")
            .expect("stream open");
        if pred(&evt) {
            return evt;
        }
    }
    panic!("no matching event within 20 reads");
}

/// 轮询 journal 直到出现满足条件的 model_change 行（控制请求按帧处理，
/// 写入与驱动 ack 之间有进程间延迟）。
fn wait_model_change(journal: &PathBuf, needle: &str) -> String {
    for _ in 0..100 {
        if let Ok(content) = std::fs::read_to_string(journal) {
            for line in content.lines() {
                if line.contains("model_change") && line.contains(needle) {
                    return line.to_string();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("no model_change line matching {needle:?} in {}", journal.display());
}

/// spawn outcome 携带内置别名表 + "default" current（spec 场景「Claude
/// session exposes the alias table」的拼装半边）。
#[tokio::test]
async fn spawn_outcome_carries_builtin_alias_table_and_default_current() {
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    let outcome = mgr
        .resume_session(
            "claude",
            vec![fake().to_str().unwrap().to_string()],
            None,
            vec![],
            // resume 语义进 SpawnOutcome 返回通道；fake 无 --resume-fails
            // 时照常接受。
            &uuid::Uuid::new_v4().to_string(),
            None,
        )
        .await
        .expect("spawn");
    let model = outcome
        .model
        .expect("claude driver must report a model surface at handshake");
    assert_eq!(
        model,
        AcpModelInfo {
            current: "default".into(),
            options: vec![
                "default".into(),
                "opus".into(),
                "sonnet".into(),
                "haiku".into()
            ],
        }
    );
    // manager 快照通道（get_model_info）零改动复用（D6）。
    let snap = mgr.get_model_info(&outcome.session_id).await.expect("live session");
    assert_eq!(snap.options.len(), 4);
}

/// 配置覆盖（`ClaudeDriver::with_models`）整体替换内置表（spec 场景
/// 「configuration overrides the alias table」的驱动半边）。
#[tokio::test]
async fn driver_models_override_replaces_builtin_table() {
    let mut agents = HashMap::new();
    agents.insert(
        "claude".to_string(),
        AgentEntry {
            driver: Arc::new(ClaudeDriver::with_models(vec!["sonnet[1m]".to_string()])),
            startup_timeout: Duration::from_secs(30),
        },
    );
    let mgr = SessionManager::new("claude".to_string(), agents);
    let outcome = mgr
        .resume_session(
            "claude",
            vec![fake().to_str().unwrap().to_string()],
            None,
            vec![],
            &uuid::Uuid::new_v4().to_string(),
            None,
        )
        .await
        .expect("spawn");
    let model = outcome.model.expect("override must still surface a model");
    assert_eq!(
        model.options,
        vec!["sonnet[1m]".to_string()],
        "non-empty override replaces the builtin table wholesale"
    );
    assert_eq!(model.current, "default");
}

/// 帧观察：首个 system init 帧的 model 名（fake 报 "fake"）经 ModelChanged
/// 把快照 current 从 "default" 覆盖为观察值（spec 场景「Claude switch
/// applies from the next turn」的观察半边 + D5 覆盖次序）。
#[tokio::test]
async fn first_turn_frames_observe_model_and_emit_model_changed() {
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    let id = mgr
        .create_claude_session(fake().to_str().unwrap(), vec![], None, vec![], "".into())
        .await
        .expect("spawn");

    mgr.send(
        &id,
        AcpCommand::CreateSession {
            session_id: id.clone(),
            prompt: "hello".into(),
        },
    )
    .await
    .expect("prompt");
    let evt = drain_until(&mgr, &id, |e| matches!(e, AcpEvent::ModelChanged { .. })).await;
    match evt {
        AcpEvent::ModelChanged { model_id, .. } => {
            assert_eq!(
                model_id, "fake",
                "the init frame's model name must overwrite the spawn-time default"
            );
        }
        other => panic!("expected ModelChanged, got {other:?}"),
    }
    // 回合照常收尾：观察不破坏既有事件流。
    let evt = drain_until(&mgr, &id, |e| matches!(e, AcpEvent::Finished { .. })).await;
    assert!(matches!(evt, AcpEvent::Finished { .. }));
}

/// SetModel 全链（spec 场景「Claude switch applies from the next turn」）：
/// 控制请求送达（journal 记 model_change）→ 乐观 ModelChanged → 后续帧
/// 覆盖纠偏（切回 "default" 后观察值把乐观值改写回 fake 的真实模型）。
#[tokio::test]
async fn set_model_switches_via_control_protocol_and_frames_correct() {
    let journal_path = journal("switch");
    let _ = std::fs::remove_file(&journal_path);
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    let id = mgr
        .create_claude_session(
            fake().to_str().unwrap(),
            vec!["--journal".into(), journal_path.to_str().unwrap().into()],
            None,
            vec![],
            "".into(),
        )
        .await
        .expect("spawn");

    // 首回合：init 帧观察 → current = "fake"。
    mgr.send(
        &id,
        AcpCommand::CreateSession {
            session_id: id.clone(),
            prompt: "hello".into(),
        },
    )
    .await
    .expect("first prompt");
    let evt = drain_until(&mgr, &id, |e| matches!(e, AcpEvent::ModelChanged { .. })).await;
    assert!(matches!(evt, AcpEvent::ModelChanged { model_id, .. } if model_id == "fake"));
    drain_until(&mgr, &id, |e| matches!(e, AcpEvent::Finished { .. })).await;

    // 切到 "opus"：SDK set_model(Some("opus")) 送达（journal）+ 乐观
    // ModelChanged。
    mgr.set_model(&id, "opus").await.expect("switch delivered");
    let evt = drain_until(&mgr, &id, |e| matches!(e, AcpEvent::ModelChanged { .. })).await;
    assert!(matches!(evt, AcpEvent::ModelChanged { model_id, .. } if model_id == "opus"));
    wait_model_change(&journal_path, "\"model\":\"opus\"");

    // 下一回合：assistant 帧报告 "opus"（fake 已切换）——观察与乐观值一致，
    // 无二次 ModelChanged，回合正常收尾。
    mgr.send(
        &id,
        AcpCommand::ContinueSession {
            session_id: id.clone(),
            prompt: "hello".into(),
        },
    )
    .await
    .expect("second prompt");
    drain_until(&mgr, &id, |e| matches!(e, AcpEvent::Finished { .. })).await;

    // 切回 "default"（SDK None）：乐观写 "default"；下一回合帧报告
    // "fake" → 观察值纠偏覆盖乐观值（D5 自愈：无失败回执协议下的诚实
    // current）。
    mgr.set_model(&id, "default").await.expect("switch back");
    let evt = drain_until(&mgr, &id, |e| matches!(e, AcpEvent::ModelChanged { .. })).await;
    assert!(matches!(evt, AcpEvent::ModelChanged { model_id, .. } if model_id == "default"));
    wait_model_change(&journal_path, "\"model\":null");

    mgr.send(
        &id,
        AcpCommand::ContinueSession {
            session_id: id.clone(),
            prompt: "hello".into(),
        },
    )
    .await
    .expect("third prompt");
    let evt = drain_until(&mgr, &id, |e| matches!(e, AcpEvent::ModelChanged { .. })).await;
    assert!(
        matches!(evt, AcpEvent::ModelChanged { model_id, .. } if model_id == "fake"),
        "frame observation must overwrite the optimistic 'default' with the agent's real model"
    );
    drain_until(&mgr, &id, |e| matches!(e, AcpEvent::Finished { .. })).await;
    let _ = std::fs::remove_file(&journal_path);
}
