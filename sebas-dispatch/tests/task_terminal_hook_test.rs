//! 会话终态 → 任务联动裁决（agent-task-center 5.1/5.2）.
//!
//! 引擎负责全部条件裁决，钩子只被调用（这里用一个记录型假钩子断言调用面）：
//! 终态 Closed/Exited、最后一轮正常完成（200）、任务的最新推进会话、有 task_id。

use sebas_acp::AcpEvent;
use sebas_channels::ChannelKey;
use sebas_dispatch::{
    DispatchHandle, Mapping, SessionMap, SessionTerminalHook, SessionTerminalTask,
};
use std::sync::Arc;
use std::sync::Mutex;

fn web_key(id: &str) -> ChannelKey {
    ChannelKey::new("web", format!("web-{id}"))
}

/// 记录型钩子：(hook, 收到的调用列表)。
fn recorder() -> (SessionTerminalHook, Arc<Mutex<Vec<SessionTerminalTask>>>) {
    let calls: Arc<Mutex<Vec<SessionTerminalTask>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = calls.clone();
    let hook: SessionTerminalHook = Arc::new(move |task| {
        sink.lock().expect("hook sink").push(task);
    });
    (hook, calls)
}

 async fn task_mapping(session_id: &str, task_id: &str, active: i64) -> Mapping {
    let mut mapping = Mapping::active(session_id);
    mapping.task_id = Some(task_id.to_string());
    mapping.project_dir = Some("/tmp/project".into());
    mapping.pending_kind = Some("claude".into());
    mapping.last_active_unix = active;
    mapping
}

#[tokio::test]
async fn closed_after_successful_turn_advances_task() {
    let map = SessionMap::new();
    let key = web_key("a");
    map.insert(key.clone(), task_mapping("sid-a", "bd-42", 100).await)
        .await
        .unwrap();
    let (router, _rx) = DispatchHandle::new(map.clone());
    let (hook, calls) = recorder();
    router.set_session_terminal_hook(hook);

    // 正常完成回合 → 结算 200。
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "sid-a".into(),
        })
        .await;

    assert_eq!(
        router.web_close_session(key).await,
        sebas_dispatch::engine::CloseOutcome::Closed {
            discarded_pending: 0
        }
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "正常完成的终态会话必须触发一次联动");
    let call = &calls[0];
    assert_eq!(call.task_id, "bd-42");
    assert_eq!(call.reason, "closed");
    assert_eq!(call.project_dir.as_deref(), Some("/tmp/project"));
    assert_eq!(call.actor.as_deref(), Some("claude"));
}

#[tokio::test]
async fn closed_without_a_completed_turn_does_not_advance() {
    let map = SessionMap::new();
    let key = web_key("b");
    map.insert(key.clone(), task_mapping("sid-b", "bd-42", 100).await)
        .await
        .unwrap();
    let (router, _rx) = DispatchHandle::new(map.clone());
    let (hook, calls) = recorder();
    router.set_session_terminal_hook(hook);

    router.web_close_session(key).await;
    assert!(calls.lock().unwrap().is_empty(), "无成功回合的关闭不动任务");
}

#[tokio::test]
async fn older_session_terminal_does_not_advance() {
    let map = SessionMap::new();
    let older = web_key("old");
    let newer = web_key("new");
    map.insert(older.clone(), task_mapping("sid-old", "bd-42", 100).await)
        .await
        .unwrap();
    map.insert(newer.clone(), task_mapping("sid-new", "bd-42", 300).await)
        .await
        .unwrap();
    let (router, _rx) = DispatchHandle::new(map.clone());
    let (hook, calls) = recorder();
    router.set_session_terminal_hook(hook);

    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "sid-old".into(),
        })
        .await;
    router.web_close_session(older).await;
    assert!(
        calls.lock().unwrap().is_empty(),
        "老会话终结不得回改任务状态（最新会话仍在）"
    );
}

#[tokio::test]
async fn failed_terminal_does_not_advance() {
    let map = SessionMap::new();
    let key = web_key("c");
    map.insert(key.clone(), task_mapping("sid-c", "bd-42", 100).await)
        .await
        .unwrap();
    let (router, _rx) = DispatchHandle::new(map.clone());
    let (hook, calls) = recorder();
    router.set_session_terminal_hook(hook);

    // terminal Error = Failed 终态：不触发联动（也把最后一轮结算成 500）。
    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "sid-c".into(),
            message: "agent crashed".into(),
            terminal: true,
        })
        .await;
    // 显式关闭退役后的映射同样不得因旧 200 误触（这里最后一轮是 500）。
    router.web_close_session(key).await;
    assert!(calls.lock().unwrap().is_empty(), "Failed 终态不动任务");
}

#[tokio::test]
async fn cancelled_last_turn_does_not_advance() {
    let map = SessionMap::new();
    let key = web_key("d");
    map.insert(key.clone(), task_mapping("sid-d", "bd-42", 100).await)
        .await
        .unwrap();
    let (router, _rx) = DispatchHandle::new(map.clone());
    let (hook, calls) = recorder();
    router.set_session_terminal_hook(hook);

    // 在飞回合（接收回执相位）→ 操作者取消 → Finished 结算 499。
    router
        .seed_card("sid-d".to_string(), "go".into())
        .await;
    assert!(
        matches!(
            router.web_cancel_session(&key).await,
            sebas_dispatch::engine::CancelOutcome::Dispatched
        ),
        "前置：取消已派发"
    );
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "sid-d".into(),
        })
        .await;
    router.web_close_session(key).await;
    assert!(
        calls.lock().unwrap().is_empty(),
        "被取消的最后一轮不是正常完成——关闭不得推进任务"
    );
}
