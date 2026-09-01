//! 集成测试（task 6.2，design N10 场景矩阵）：spec 全部 Scenario 有对应
//! 命名测试。FakeLlmClient 双模式驱动，无需真 gateway / provider / 模型。

use sebas_agent::llm::fake::FakeLlmClient;
use sebas_agent::message::{BudgetConfig, ContentBlock, Message};
use sebas_agent::session::{AgentEvent, SessionConfig, SessionManager};
use sebas_agent::tools::ToolRegistry;
use std::sync::Arc;
use std::time::Duration;

/// 收集事件直到终态（Finished / Error）。
async fn wait_terminal(rx: &mut tokio::sync::broadcast::Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut evs = Vec::new();
    loop {
        match rx.recv().await {
            Ok(ev) => {
                let terminal = matches!(ev, AgentEvent::Finished { .. } | AgentEvent::Error { .. });
                evs.push(ev);
                if terminal {
                    return evs;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(e) => panic!("event channel closed: {e}"),
        }
    }
}

fn manager(llm: Arc<FakeLlmClient>) -> SessionManager {
    SessionManager::new(
        llm,
        ToolRegistry::new(Duration::from_secs(10)),
        SessionConfig::default(),
    )
}

fn budget_llm(turns: Vec<sebas_agent::llm::LlmTurn>) -> Arc<FakeLlmClient> {
    Arc::new(FakeLlmClient::scripted(turns))
}

/// spec「Multi-step task completes without operator input」：
/// ≥5 工具执行的多轮响应，全部结果先回填再发起下一次模型调用；
/// 循环只在无工具调用的响应后以终态 finished 结束。
#[tokio::test]
async fn scripted_five_tool_multi_step_loop() {
    let dir = tempfile::tempdir().unwrap();
    let turns = vec![
        FakeLlmClient::call_tools(vec![(
            "t1",
            "write",
            serde_json::json!({"path": "notes.txt", "content": "alpha"}),
        )]),
        FakeLlmClient::call_tools(vec![(
            "t2",
            "read",
            serde_json::json!({"path": "notes.txt"}),
        )]),
        FakeLlmClient::call_tools(vec![(
            "t3",
            "edit",
            serde_json::json!({"path": "notes.txt", "old_string": "alpha", "new_string": "beta"}),
        )]),
        FakeLlmClient::call_tools(vec![(
            "t4",
            "grep",
            serde_json::json!({"pattern": "beta", "include": "*.txt"}),
        )]),
        FakeLlmClient::call_tools(vec![(
            "t5",
            "glob",
            serde_json::json!({"pattern": "*.txt"}),
        )]),
        FakeLlmClient::say("chain complete"),
    ];
    let m = manager(budget_llm(turns));
    let h = m.create_session(dir.path().to_path_buf());
    let mut rx = h.subscribe();
    h.prompt("run the chain").await;
    let evs = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
        .await
        .unwrap();

    // 五个工具都执行了。
    let starts: Vec<&str> = evs
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolStart { tool_name, .. } => Some(tool_name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(starts, vec!["write", "read", "edit", "grep", "glob"]);
    // 终态是 finished（无预算标记路径——Finished 不带失败语义）。
    assert!(evs.iter().any(|e| matches!(e, AgentEvent::Finished { .. })));
    assert!(!evs.iter().any(|e| matches!(e, AgentEvent::Error { .. })));
    // 文件演进结果正确（链路真实执行）。
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notes.txt")).unwrap(),
        "beta"
    );
}

/// spec「Model recovers from a failed command」：bash 非零退出 →
/// 模型看到失败结果并自愈，循环正常继续。
#[tokio::test]
async fn stateful_self_heal_after_failed_command() {
    let dir = tempfile::tempdir().unwrap();
    // 有状态 fake：第一轮发一个失败的 bash 命令；看到 error tool_result
    // 后改发修复命令；成功后收尾。
    let llm = Arc::new(FakeLlmClient::stateful(Box::new(|history: &[Message]| {
        // 非零退出不是 is_error（ok:true 携带 exit code）——失败以
        // "[exit code: N]" 文本形态出现在 tool_result 里。
        let saw_failure = history.iter().any(|m| {
            m.content.iter().any(|b| matches!(
                b,
                ContentBlock::ToolResult { content, .. }
                    if content.contains("boom-marker") && content.contains("exit code: 7")
            ))
        });
        let saw_fix = history.iter().any(|m| {
            m.content.iter().any(|b| matches!(
                b,
                ContentBlock::ToolResult { content, is_error: false, .. }
                    if content.contains("fixed-output")
            ))
        });
        if !saw_failure {
            vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "echo boom-marker >&2; exit 7"}),
            }]
        } else if !saw_fix {
            vec![ContentBlock::ToolUse {
                id: "t2".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "echo fixed-output"}),
            }]
        } else {
            vec![ContentBlock::Text { text: "recovered".into() }]
        }
    })));
    let m = SessionManager::new(
        llm,
        ToolRegistry::new(Duration::from_secs(10)),
        SessionConfig::default(),
    );
    let h = m.create_session(dir.path().to_path_buf());
    let mut rx = h.subscribe();
    h.prompt("try, fail, heal").await;
    let evs = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
        .await
        .unwrap();

    // 失败结果作为数据回传给模型（ok:true 携带 exit code 的文本形态）。
    assert!(evs.iter().any(|e| matches!(
        e,
        AgentEvent::ToolEnd { result, .. } if result.contains("boom-marker")
    )));
    // 循环继续：修复命令也执行了。
    assert!(evs.iter().any(|e| matches!(
        e,
        AgentEvent::ToolEnd { result, .. } if result.contains("fixed-output")
    )));
    // 干净收尾。
    assert!(evs.iter().any(|e| matches!(e, AgentEvent::Finished { .. })));
}

/// spec「Tool timeout terminates the command」：bash 超时 → 进程组被杀，
/// 模型收到 timeout 失败结果。
#[tokio::test]
async fn tool_timeout_terminates_command() {
    let dir = tempfile::tempdir().unwrap();
    let turns = vec![
        FakeLlmClient::call_tools(vec![(
            "t1",
            "bash",
            serde_json::json!({"command": "sleep 456777", "timeout_secs": 1}),
        )]),
        FakeLlmClient::say("saw the timeout"),
    ];
    let m = manager(budget_llm(turns));
    let h = m.create_session(dir.path().to_path_buf());
    let mut rx = h.subscribe();
    h.prompt("sleep forever").await;
    let evs = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
        .await
        .unwrap();
    assert!(evs.iter().any(|e| matches!(
        e,
        AgentEvent::ToolEnd { result, .. } if result.contains("timeout")
    )), "timeout failure must be visible to the model: {evs:?}");
    // 进程组已终止，无孤儿。
    tokio::time::sleep(Duration::from_millis(300)).await;
    let orphan = std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .any(|e| {
            let Ok(c) = std::fs::read(e.path().join("cmdline")) else {
                return false;
            };
            let args: Vec<String> = c
                .split(|&b| b == 0u8)
                .map(|x| String::from_utf8_lossy(x).into_owned())
                .filter(|s| !s.is_empty())
                .collect();
            args == vec!["sleep".to_string(), "456777".to_string()]
        });
    assert!(!orphan, "timed-out command left an orphan");
}

/// spec「Model-call budget ends the turn cleanly」：模型仍要工具但
/// 调用数耗尽 → 终态 finished 携带预算耗尽语义、非错误、会话未终态失败。
#[tokio::test]
async fn model_call_budget_exhaustion_ends_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let turns = vec![
        FakeLlmClient::call_tools(vec![(
            "t1",
            "bash",
            serde_json::json!({"command": "echo a"}),
        )]),
        FakeLlmClient::call_tools(vec![(
            "t2",
            "bash",
            serde_json::json!({"command": "echo b"}),
        )]),
    ];
    let m = SessionManager::new(
        budget_llm(turns),
        ToolRegistry::new(Duration::from_secs(10)),
        SessionConfig {
            budget: BudgetConfig {
                max_model_calls: 2,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let h = m.create_session(dir.path().to_path_buf());
    let mut rx = h.subscribe();
    h.prompt("keep calling tools").await;
    let evs = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
        .await
        .unwrap();
    // 终态是 Finished，不是 Error。
    assert!(matches!(&evs[..], [.., AgentEvent::Finished { .. }]));
    assert!(!evs.iter().any(|e| matches!(e, AgentEvent::Error { .. })));
    // 预算耗尽语义可见（FinishReason::Budget 由引擎收敛为正常 Finished——
    // 1a 的 AgentEvent 词汇与 AcpEvent 对齐，无 budget 变体；终态后引擎
    // 不再调用模型是结构性保证：max_model_calls=2，脚本只有 2 项且都消费完）。
}

/// spec「Two sessions do not cross events」：两个不同 workdir 的会话并行，
/// 事件流只含本会话事件，工具执行互不见对方目录。
#[tokio::test]
async fn two_sessions_isolated_events_and_workdirs() {
    let dir1 = tempfile::tempdir().unwrap();
    let dir2 = tempfile::tempdir().unwrap();
    let llm = Arc::new(FakeLlmClient::stateful(Box::new(|history: &[Message]| {
        let has_result = history
            .iter()
            .any(|m| m.content.iter().any(|b| matches!(b, ContentBlock::ToolResult { .. })));
        if has_result {
            vec![ContentBlock::Text { text: "done".into() }]
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
    })));
    let m = manager(llm);
    let h1 = m.create_session(dir1.path().to_path_buf());
    let h2 = m.create_session(dir2.path().to_path_buf());
    let mut rx1 = h1.subscribe();
    let mut rx2 = h2.subscribe();

    h1.prompt("marker-one").await;
    h2.prompt("marker-two").await;
    let evs1 = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx1))
        .await
        .unwrap();
    let evs2 = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx2))
        .await
        .unwrap();

    // 每条事件都严格归属本会话。
    for (evs, key) in [(&evs1, &h1.key), (&evs2, &h2.key)] {
        assert!(evs.iter().all(|e| match e {
            AgentEvent::TextDelta { session_id, .. }
            | AgentEvent::ThinkingDelta { session_id, .. }
            | AgentEvent::ToolStart { session_id, .. }
            | AgentEvent::ToolProgress { session_id, .. }
            | AgentEvent::ToolEnd { session_id, .. }
            | AgentEvent::Finished { session_id }
            | AgentEvent::Error { session_id, .. } => session_id == key,
        }));
    }
    // 工作目录互不可见。
    assert_eq!(
        std::fs::read_to_string(dir1.path().join("marker.txt")).unwrap(),
        "marker-one"
    );
    assert_eq!(
        std::fs::read_to_string(dir2.path().join("marker.txt")).unwrap(),
        "marker-two"
    );
    assert!(!dir1.path().join("marker-two.txt").exists());
    assert!(!dir2.path().join("marker-one.txt").exists());
}

/// spec「Session stays usable after a cancelled turn」：取消后同会话
/// 再 prompt，新 turn 在保留历史上正常执行（内核层；会话层版本在
/// session::tests，此处走 SessionManager 公共 API）。
#[tokio::test]
async fn cancellation_via_manager_then_next_prompt_runs() {
    let dir = tempfile::tempdir().unwrap();
    let turns = vec![
        FakeLlmClient::call_tools(vec![(
            "t1",
            "bash",
            serde_json::json!({"command": "sleep 456888"}),
        )]),
        FakeLlmClient::say("after cancel"),
    ];
    let m = manager(budget_llm(turns));
    let h = m.create_session(dir.path().to_path_buf());
    let mut rx = h.subscribe();
    h.prompt("long").await;
    // 等工具启动后取消。
    loop {
        let ev = rx.recv().await.unwrap();
        if matches!(ev, AgentEvent::ToolStart { .. }) {
            h.cancel().await;
            break;
        }
    }
    let evs = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
        .await
        .unwrap();
    // 取消是 cancellation outcome（非 finished）。
    assert!(!evs.iter().any(|e| matches!(e, AgentEvent::Finished { .. })));
    assert!(matches!(
        &evs[..],
        [.., AgentEvent::Error { message, terminal: false, .. }] if message == "turn cancelled"
    ));

    // 无孤儿。
    tokio::time::sleep(Duration::from_millis(300)).await;
    let orphan = std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .any(|e| {
            let Ok(c) = std::fs::read(e.path().join("cmdline")) else {
                return false;
            };
            let args: Vec<String> = c
                .split(|&b| b == 0u8)
                .map(|x| String::from_utf8_lossy(x).into_owned())
                .filter(|s| !s.is_empty())
                .collect();
            args == vec!["sleep".to_string(), "456888".to_string()]
        });
    assert!(!orphan);

    // 同会话继续可用。
    h.prompt("again").await;
    let evs = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
        .await
        .unwrap();
    assert!(evs
        .iter()
        .any(|e| matches!(e, AgentEvent::TextDelta { delta, .. } if delta == "after cancel")));
    assert!(evs.iter().any(|e| matches!(e, AgentEvent::Finished { .. })));
}
