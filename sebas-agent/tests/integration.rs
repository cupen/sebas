//! 集成测试（task 6.2）：经 SessionManager 公开 API 驱动的端到端场景。
//!
//! 对应 spec 场景（每个测试名可追溯到 agent-core spec）：
//! - `multi_step_scripted_loop_finishes_cleanly`  → Turn loop / multi-step（C1）
//! - `stateful_self_heal_after_nonzero_exit`      → bash / Model recovers（C4）
//! - `cancel_mid_bash_then_session_reusable`      → Cancellation safety（C7）
//! - `budget_exhaustion_ends_as_finished`         → Turn budgets（C8）
//! - `two_sessions_isolated`                      → Session lifecycle（5.1）
//! - `event_vocabulary_over_full_turn`            → Streaming vocabulary（C2）

use sebas_agent::llm::fake::FakeLlmClient;
use sebas_agent::message::{BudgetConfig, ContentBlock, Message};
use sebas_agent::session::{AgentEvent, SessionConfig, SessionManager};
use sebas_agent::tools::ToolRegistry;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

async fn wait_terminal(rx: &mut broadcast::Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut evs = Vec::new();
    loop {
        match rx.recv().await {
            Ok(ev) => {
                let terminal =
                    matches!(ev, AgentEvent::Finished { .. } | AgentEvent::Error { .. });
                evs.push(ev);
                if terminal {
                    return evs;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(e) => panic!("event channel closed: {e}"),
        }
    }
}

fn timeout() -> Duration {
    Duration::from_secs(60)
}

/// 脚本化 ≥5 工具调用多步 turn，干净收尾（spec：multi-step scenario）。
#[tokio::test]
async fn multi_step_scripted_loop_finishes_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let llm = FakeLlmClient::scripted(vec![
        FakeLlmClient::call_tools(vec![(
            "t1",
            "write",
            serde_json::json!({"path": "notes.txt", "content": "alpha beta"}),
        )]),
        FakeLlmClient::call_tools(vec![(
            "t2",
            "read",
            serde_json::json!({"path": "notes.txt"}),
        )]),
        FakeLlmClient::call_tools(vec![(
            "t3",
            "edit",
            serde_json::json!({"path": "notes.txt", "old_string": "beta", "new_string": "beta gamma"}),
        )]),
        FakeLlmClient::call_tools(vec![(
            "t4",
            "grep",
            serde_json::json!({"pattern": "gamma"}),
        )]),
        FakeLlmClient::call_tools(vec![(
            "t5",
            "glob",
            serde_json::json!({"pattern": "*.txt"}),
        )]),
        FakeLlmClient::say("all five tools ran; summary complete"),
    ]);
    let manager = SessionManager::new(
        Arc::new(llm),
        ToolRegistry::new(Duration::from_secs(10)),
        SessionConfig::default(),
    );
    let handle = manager.create_session(dir.path().to_path_buf());
    let mut rx = handle.subscribe();
    handle.prompt("do the chained task").await;
    let evs = tokio::time::timeout(timeout(), wait_terminal(&mut rx))
        .await
        .unwrap();

    // 五个工具都执行了
    let tools: Vec<&String> = evs
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolStart { tool_name, .. } => Some(tool_name),
            _ => None,
        })
        .collect();
    assert_eq!(
        tools,
        vec!["write", "read", "edit", "grep", "glob"],
        "five tool executions in order"
    );
    // 文件链式演进的最终状态
    let content = std::fs::read_to_string(dir.path().join("notes.txt")).unwrap();
    assert_eq!(content, "alpha beta gamma");
    // 干净收尾：Finished（而非 Error）
    assert!(
        evs.iter()
            .any(|e| matches!(e, AgentEvent::Finished { .. })),
        "must end with a finished event"
    );
    assert!(!evs.iter().any(|e| matches!(e, AgentEvent::Error { .. })));
    // 最终文本在所有工具结果回填之后才出现
    let last_text = evs
        .iter()
        .filter_map(|e| match e {
            AgentEvent::TextDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
.next_back()
        .unwrap();
    assert_eq!(last_text, "all five tools ran; summary complete");
}

/// 有状态 fake：bash 退出非零 → 模型看到 `[exit code: N]` 后自愈（spec：
/// Model recovers from a failed command；C4 错误即数据）。
#[tokio::test]
async fn stateful_self_heal_after_nonzero_exit() {
    let dir = tempfile::tempdir().unwrap();
    let llm = FakeLlmClient::stateful(Box::new(|history: &[Message]| {
        // 按最近一条 tool_result 决策：失败 → 自愈命令；已自愈 → 收尾文本。
        let last_result = history.iter().rev().find_map(|m| {
            m.content.iter().find_map(|b| match b {
                ContentBlock::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
        });
        match last_result {
            Some(c) if c.contains("[exit code: 3]") => vec![ContentBlock::ToolUse {
                id: "t2".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "echo recovered"}),
            }],
            Some(c) if c.contains("recovered") => vec![ContentBlock::Text {
                text: "self-healed".into(),
            }],
            _ => vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "exit 3"}),
            }],
        }
    }));
    let manager = SessionManager::new(
        Arc::new(llm),
        ToolRegistry::new(Duration::from_secs(10)),
        SessionConfig::default(),
    );
    let handle = manager.create_session(dir.path().to_path_buf());
    let mut rx = handle.subscribe();
    handle.prompt("run something that fails first").await;
    let evs = tokio::time::timeout(timeout(), wait_terminal(&mut rx))
        .await
        .unwrap();

    // 循环正常继续：失败结果回填后模型改发成功命令并收尾
    let ends: Vec<String> = evs
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolEnd { result, .. } => Some(result.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(ends.len(), 2, "two bash executions");
    assert!(ends[0].contains("[exit code: 3]"), "failure visible to model");
    assert!(ends[1].contains("recovered"));
    assert!(
        evs.iter()
            .any(|e| matches!(e, AgentEvent::TextDelta { delta, .. } if delta == "self-healed"))
    );
    assert!(!evs.iter().any(|e| matches!(e, AgentEvent::Error { .. })));
}

/// 取消打断长 bash：进程组被终止、取消结局（非 finished）、会话可继续用
///（spec：cancellation scenarios；C7）。
#[tokio::test]
async fn cancel_mid_bash_then_session_reusable() {
    let dir = tempfile::tempdir().unwrap();
    let llm = FakeLlmClient::scripted(vec![
        FakeLlmClient::call_tools(vec![(
            "t1",
            "bash",
            serde_json::json!({"command": "echo begun; sleep 987654"}),
        )]),
        FakeLlmClient::say("turn after cancel"),
    ]);
    let manager = SessionManager::new(
        Arc::new(llm),
        ToolRegistry::new(Duration::from_secs(60)),
        SessionConfig::default(),
    );
    let handle = manager.create_session(dir.path().to_path_buf());
    let mut rx = handle.subscribe();

    handle.prompt("long running").await;
    loop {
        let ev = rx.recv().await.unwrap();
        if matches!(ev, AgentEvent::ToolStart { .. }) {
            handle.cancel().await;
            break;
        }
    }
    let evs = tokio::time::timeout(timeout(), wait_terminal(&mut rx))
        .await
        .unwrap();
    // 取消结局而非 finished（spec：emits a cancellation outcome）；
    // ToolEnd（bash 返回 cancelled 错误结果）在 Error 之前。
    assert!(
        !evs.iter().any(|e| matches!(e, AgentEvent::Finished { .. })),
        "cancellation must not end as finished"
    );
    match evs.last() {
        Some(AgentEvent::Error {
            message, terminal, ..
        }) => {
            assert_eq!(message, "turn cancelled");
            assert!(!*terminal);
        }
        other => panic!("expected cancellation outcome, got {other:?}"),
    }

    // 同一会话下一个 prompt 正常执行（history 保留之上）
    handle.prompt("again").await;
    let evs = tokio::time::timeout(timeout(), wait_terminal(&mut rx))
        .await
        .unwrap();
    assert!(evs.iter().any(
        |e| matches!(e, AgentEvent::TextDelta { delta, .. } if delta == "turn after cancel")
    ));
    assert!(evs.iter().any(|e| matches!(e, AgentEvent::Finished { .. })));
}

/// 预算耗尽以正常 finished 收尾并携带预算标记，会话未标记 terminal 失败
///（spec：budgets scenario；C8）。模型调用预算 2：模型仍要工具时被拦截。
#[tokio::test]
async fn budget_exhaustion_ends_as_finished() {
    let dir = tempfile::tempdir().unwrap();
    let llm = FakeLlmClient::scripted(vec![
        FakeLlmClient::call_tools(vec![(
            "t1",
            "bash",
            serde_json::json!({"command": "echo one"}),
        )]),
        FakeLlmClient::call_tools(vec![(
            "t2",
            "bash",
            serde_json::json!({"command": "echo two"}),
        )]),
    ]);
    let config = SessionConfig {
        budget: BudgetConfig {
            max_model_calls: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    let manager = SessionManager::new(
        Arc::new(llm),
        ToolRegistry::new(Duration::from_secs(10)),
        config,
    );
    let handle = manager.create_session(dir.path().to_path_buf());
    let mut rx = handle.subscribe();
    handle.prompt("keep calling tools").await;
    let evs = tokio::time::timeout(timeout(), wait_terminal(&mut rx))
        .await
        .unwrap();

    // 两轮工具执行后模型调用被拦截
    assert_eq!(
        evs.iter()
            .filter(|e| matches!(e, AgentEvent::ToolEnd { .. }))
            .count(),
        2
    );
    // 终态是 finished 而非 error
    assert!(
        evs.iter()
            .any(|e| matches!(e, AgentEvent::Finished { .. })),
        "budget exhaustion must be a finished outcome, got {evs:?}"
    );
    assert!(!evs.iter().any(|e| matches!(e, AgentEvent::Error { .. })));
}

/// 两个会话并发：事件按 session_id 隔离，工作目录互不可见
///（spec：session lifecycle scenarios）。
#[tokio::test]
async fn two_sessions_isolated() {
    let dir1 = tempfile::tempdir().unwrap();
    let dir2 = tempfile::tempdir().unwrap();
    // 有状态 fake：按 prompt 文本写各自 marker 文件
    let llm = FakeLlmClient::stateful(Box::new(|history: &[Message]| {
        let has_result = history.iter().any(|m| {
            m.content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolResult { .. }))
        });
        if has_result {
            vec![ContentBlock::Text {
                text: "done".into(),
            }]
        } else {
            let prompt = match history.first().map(|m| &m.content[0]) {
                Some(ContentBlock::Text { text }) => text.clone(),
                _ => "unknown".into(),
            };
            vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "write".into(),
                input: serde_json::json!({"path": "marker.txt", "content": prompt}),
            }]
        }
    }));
    let manager = SessionManager::new(
        Arc::new(llm),
        ToolRegistry::new(Duration::from_secs(10)),
        SessionConfig::default(),
    );
    let h1 = manager.create_session(dir1.path().to_path_buf());
    let h2 = manager.create_session(dir2.path().to_path_buf());
    let mut rx1 = h1.subscribe();
    let mut rx2 = h2.subscribe();

    h1.prompt("one").await;
    h2.prompt("two").await;
    let evs1 = tokio::time::timeout(timeout(), wait_terminal(&mut rx1))
        .await
        .unwrap();
    let evs2 = tokio::time::timeout(timeout(), wait_terminal(&mut rx2))
        .await
        .unwrap();

    // 事件流只含自己 session_id 的事件
    for (evs, key) in [(&evs1, &h1.key), (&evs2, &h2.key)] {
        assert!(!evs.is_empty());
        for ev in evs {
            let v = serde_json::to_value(ev).unwrap();
            let sid = v["session_id"].as_str().unwrap();
            assert_eq!(sid, *key, "events must not cross sessions");
        }
    }
    // 工具执行互不可见对方工作目录
    assert_eq!(
        std::fs::read_to_string(dir1.path().join("marker.txt")).unwrap(),
        "one"
    );
    assert_eq!(
        std::fs::read_to_string(dir2.path().join("marker.txt")).unwrap(),
        "two"
    );
}

/// 事件词汇全 turn 断言（spec：streaming vocabulary；C2）：
/// 词汇 = AcpEvent 镜像（无 PermissionRequest），顺序 = delta 到达序。
#[tokio::test]
async fn event_vocabulary_over_full_turn() {
    let dir = tempfile::tempdir().unwrap();
    let llm = FakeLlmClient::scripted(vec![
        FakeLlmClient::call_tools(vec![(
            "t1",
            "write",
            serde_json::json!({"path": "a.txt", "content": "x"}),
        )]),
        FakeLlmClient::say("final answer"),
    ]);
    let manager = SessionManager::new(
        Arc::new(llm),
        ToolRegistry::new(Duration::from_secs(10)),
        SessionConfig::default(),
    );
    let handle = manager.create_session(dir.path().to_path_buf());
    let mut rx = handle.subscribe();
    handle.prompt("go").await;
    let evs = tokio::time::timeout(timeout(), wait_terminal(&mut rx))
        .await
        .unwrap();

    let kinds: Vec<String> = evs
        .iter()
        .map(|e| {
            serde_json::to_value(e).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "tool_start",
            "tool_end",
            "tool_finish",
            "text_delta",
            "session_summary",
            "finished",
        ]
    );
    // serde 形状与 AcpEvent 兼容（type tag + snake_case + 同名字段）
    let v = serde_json::to_value(&evs[0]).unwrap();
    assert!(v.get("session_id").is_some());
    assert!(v.get("tool_name").is_some());
    assert!(v.get("args").is_some());
}
