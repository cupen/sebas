//! 会话管理（task 5.1–5.3，design N4/N7）：SessionManager / SessionHandle /
//! 事件词汇 / 系统提示词装配。
//!
//! 每会话一个 tokio task：等待 Prompt → pin turn future → select 命令通道
//! （Cancel 生效、Prompt 排队、通道关闭取消）→ 发射终态 → 处理队列。
//! 会话为内存态，进程退出即失（OQ1 延后持久化）。

use crate::llm::LlmClient;
use crate::loop_::{TurnEngine, TurnEmit, TurnOutcome};
use crate::message::{BudgetConfig, Message};
use crate::policy::{ApprovalAnswer, Approver, PolicyEngine};
use crate::tools::{ToolCtx, ToolRegistry};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

/// 会话事件词汇：与 `acp-claude::AcpEvent` 一一对应（serde 形状兼容：
/// `type` tag + snake_case）。Phase 2 在此基础上**启用** `PermissionRequest`
/// （策略审批面，1a 刻意不发）并新增 `ToolPolicy`（策略结果事件）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    TextDelta {
        session_id: String,
        delta: String,
    },
    ThinkingDelta {
        session_id: String,
        delta: String,
    },
    ToolStart {
        session_id: String,
        tool_name: String,
        args: serde_json::Value,
    },
    ToolProgress {
        session_id: String,
        tool_name: String,
        progress: String,
    },
    ToolEnd {
        session_id: String,
        tool_name: String,
        result: String,
    },
    /// 策略审批请求（request_id == tool_use_id，permission-flow 关联契约）。
    /// 消费端（webui 审查卡 / 飞书卡）呈现后经 `SessionHandle::answer_permission`
    /// 回填决定；无回答者/超时 = fail closed 拒绝。
    PermissionRequest {
        session_id: String,
        request_id: String,
        tool_name: String,
        args: serde_json::Value,
        reason: String,
    },
    /// 策略流结果事件：outcome ∈ allowed_once | allowed_session | escalated |
    /// denied | unavailable。让消费端区分「策略拒绝」与「工具崩溃」。
    ToolPolicy {
        session_id: String,
        request_id: String,
        tool_name: String,
        outcome: String,
    },
    /// 工具结构化收尾事件（Phase 2，design N6）：`ToolEnd` 的结构化孪生，
    /// 供 SSE/日志消费结构化字段而无需解析 result 文本。
    ToolFinish {
        session_id: String,
        tool_name: String,
        ok: bool,
        truncated: bool,
        exit_code: Option<i32>,
    },
    /// turn 汇总（Phase 2，design N6）：引擎在 turn 收尾时发射（Finished /
    /// Cancelled / Failed 均发），供宿主记录与展示。
    SessionSummary {
        session_id: String,
        model_calls: u32,
        tool_calls: u32,
        turn_ms: u64,
    },
    Finished {
        session_id: String,
    },
    Error {
        session_id: String,
        message: String,
        #[serde(default)]
        terminal: bool,
    },
}

/// 会话命令。
#[derive(Debug)]
enum SessionCmd {
    Prompt(String),
    Cancel,
    /// 审批决定回填（webui 审查卡 / agent-dev 脚本通道）。
    PermissionAnswer {
        request_id: String,
        answer: ApprovalAnswer,
    },
}

/// 会话级配置。`model` 缺省值为占位（OQ3 延后：模型默认值由部署方定）。
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub model: String,
    pub budget: BudgetConfig,
    pub bash_timeout: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-5".into(),
            budget: BudgetConfig::default(),
            bash_timeout: Duration::from_secs(120),
        }
    }
}

/// 会话管理器：每会话一个 tokio task（design N4）。
/// 策略引擎与审批回答者是会话级共享件（None = 不做策略门控，1a 行为）。
pub struct SessionManager {
    llm: Arc<dyn LlmClient>,
    registry: Arc<ToolRegistry>,
    config: SessionConfig,
    policy: Option<Arc<PolicyEngine>>,
    approver: Option<Arc<dyn Approver>>,
}

impl SessionManager {
    pub fn new(llm: Arc<dyn LlmClient>, registry: ToolRegistry, config: SessionConfig) -> Self {
        Self {
            llm,
            registry: Arc::new(registry),
            config,
            policy: None,
            approver: None,
        }
    }

    /// 挂策略引擎（task 1.1–1.3：所有工具调用先过策略）。
    pub fn with_policy(mut self, policy: Arc<PolicyEngine>) -> Self {
        self.policy = Some(policy);
        self
    }

    /// 挂审批回答者（webui 审查卡 / agent-dev 脚本应答 / 测试桩）。
    pub fn with_approver(mut self, approver: Arc<dyn Approver>) -> Self {
        self.approver = Some(approver);
        self
    }

    /// 创建绑定 `workdir` 的会话（多会话并发，彼此隔离）。
    pub fn create_session(&self, workdir: PathBuf) -> SessionHandle {
        let key = uuid::Uuid::new_v4().to_string();
        let (cmd_tx, cmd_rx) = mpsc::channel::<SessionCmd>(64);
        let (evt_tx, _) = broadcast::channel::<AgentEvent>(1024);
        let task = SessionTask {
            llm: self.llm.clone(),
            registry: self.registry.clone(),
            config: self.config.clone(),
            policy: self.policy.clone(),
            approver: self.approver.clone(),
            workdir,
            key: key.clone(),
            cmd_rx,
            evt_tx: evt_tx.clone(),
        };
        tokio::spawn(task.run());
        SessionHandle {
            key,
            cmd_tx,
            evt_tx,
        }
    }
}

/// 会话句柄（Clone：多端可同时持有）。
#[derive(Clone)]
pub struct SessionHandle {
    pub key: String,
    cmd_tx: mpsc::Sender<SessionCmd>,
    evt_tx: broadcast::Sender<AgentEvent>,
}

impl SessionHandle {
    /// 提交用户输入；turn 进行中则排队，turn 结束后按序处理（串行队列）。
    pub async fn prompt(&self, text: impl Into<String>) {
        self.cmd_tx
            .send(SessionCmd::Prompt(text.into()))
            .await
            .expect("session task died");
    }

    /// 请求取消当前 turn（空闲时无效果）。
    pub async fn cancel(&self) {
        let _ = self.cmd_tx.send(SessionCmd::Cancel).await;
    }

    /// 回填一个权限决定（design N5：`permission_decision` 回 SessionHandle）。
    /// request_id 来自 `PermissionRequest` 事件；无待决请求时静默丢弃。
    pub async fn answer_permission(&self, request_id: impl Into<String>, answer: ApprovalAnswer) {
        let _ = self
            .cmd_tx
            .send(SessionCmd::PermissionAnswer {
                request_id: request_id.into(),
                answer,
            })
            .await;
    }

    /// 订阅事件流（每订阅者独立游标）。
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.evt_tx.subscribe()
    }
}

struct SessionTask {
    llm: Arc<dyn LlmClient>,
    registry: Arc<ToolRegistry>,
    config: SessionConfig,
    policy: Option<Arc<PolicyEngine>>,
    approver: Option<Arc<dyn Approver>>,
    workdir: PathBuf,
    key: String,
    cmd_rx: mpsc::Receiver<SessionCmd>,
    evt_tx: broadcast::Sender<AgentEvent>,
}

impl SessionTask {
    async fn run(mut self) {
        let engine = TurnEngine::new(self.config.budget.clone());
        let mut history: Vec<Message> = Vec::new();
        let read_files = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
        let mut pending: VecDeque<String> = VecDeque::new();

        'session: while let Some(cmd) = self.cmd_rx.recv().await {
            let mut text = match cmd {
                SessionCmd::Prompt(t) => t,
                SessionCmd::Cancel => continue,
                SessionCmd::PermissionAnswer { request_id, answer } => {
                    // 空闲期到达的审批决定：请求已随 turn 结束超时/失效，静默丢弃。
                    if let Some(a) = &self.approver {
                        let _ = a.answer(&request_id, answer);
                    }
                    continue;
                }
            };
            loop {
                let cancel = CancellationToken::new();
                let tool_ctx = ToolCtx {
                    workdir: self.workdir.clone(),
                    cancel: cancel.clone(),
                    read_files: read_files.clone(),
                    policy: self.policy.clone(),
                    approver: self.approver.clone(),
                };
                let system = build_system(&self.workdir);
                let emit = TurnEmit::new(&self.key, &self.evt_tx);
                // future（及其对 text/history 的借用）收在块作用域内，
                // 出块即释放，随后才能把队列中的下一条 prompt 赋给 text。
                let outcome = {
                    let fut = engine.run_turn(
                        self.llm.as_ref(),
                        &self.registry,
                        &tool_ctx,
                        &mut history,
                        &text,
                        &system,
                        &self.config.model,
                        cancel.clone(),
                        &emit,
                    );
                    tokio::pin!(fut);
                    // turn 进行中继续服务命令：Cancel 即时生效，Prompt 排队。
                    loop {
                        tokio::select! {
                            cmd = self.cmd_rx.recv() => match cmd {
                                Some(SessionCmd::Cancel) => cancel.cancel(),
                                Some(SessionCmd::Prompt(t)) => pending.push_back(t),
                                Some(SessionCmd::PermissionAnswer { request_id, answer }) => {
                                    // 审批决定回填 → 转交审批人（oneshot 唤醒门控等待）。
                                    if let Some(a) = &self.approver {
                                        let _ = a.answer(&request_id, answer);
                                    }
                                }
                                None => cancel.cancel(), // 管理器已丢句柄：收尾当前 turn
                            },
                            out = &mut fut => break out,
                        }
                    }
                };
                // 终态事件：取消不是 Finished（spec：cancellation outcome）。
                match outcome {
                    TurnOutcome::Finished { .. } => {
                        let _ = self.evt_tx.send(AgentEvent::Finished {
                            session_id: self.key.clone(),
                        });
                    }
                    TurnOutcome::Cancelled => {
                        let _ = self.evt_tx.send(AgentEvent::Error {
                            session_id: self.key.clone(),
                            message: "turn cancelled".into(),
                            terminal: false,
                        });
                    }
                    TurnOutcome::Failed { terminal, message } => {
                        let _ = self.evt_tx.send(AgentEvent::Error {
                            session_id: self.key.clone(),
                            message,
                            terminal,
                        });
                    }
                }
                match pending.pop_front() {
                    Some(next) => {
                        text = next;
                        continue;
                    }
                    None => continue 'session,
                }
            }
        }
    }
}

/// 系统提示词装配（task 5.2，checklist C6）：身份段 + 工作目录 +
/// AGENTS.md（在前）+ CLAUDE.md，存在才注入；两者皆无则仅基础提示词。
pub fn build_system(workdir: &std::path::Path) -> String {
    let mut s = format!(
        "你是 sebas-agent，sebas 的原生编码代理。当前工作目录：{}。\n\
         工作纪律：修改文件前先 read；工具失败时读取错误输出并自愈；不臆测文件内容；保持改动最小。",
        workdir.display()
    );
    for name in ["AGENTS.md", "CLAUDE.md"] {
        if let Ok(content) = std::fs::read_to_string(workdir.join(name))
            && !content.trim().is_empty()
        {
            s.push_str(&format!("\n\n=== {name}（项目指令） ===\n{}", content));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::fake::FakeLlmClient;
    use crate::llm::{LlmError, LlmRequest, LlmTurn, StreamEvent};
    use crate::message::ContentBlock;
    use async_trait::async_trait;

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

    #[test]
    fn system_prompt_injects_memory_files_in_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "AGENTS-MARKER-CONTENT").unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "CLAUDE-MARKER-CONTENT").unwrap();
        let s = build_system(dir.path());
        assert!(s.contains("AGENTS-MARKER-CONTENT"));
        assert!(s.contains("CLAUDE-MARKER-CONTENT"));
        assert!(
            s.find("AGENTS.md（项目指令）").unwrap() < s.find("CLAUDE.md（项目指令）").unwrap(),
            "AGENTS.md must come before CLAUDE.md"
        );
        assert!(s.contains(&dir.path().display().to_string()));
    }

    #[test]
    fn system_prompt_without_memory_files_is_base_only() {
        let dir = tempfile::tempdir().unwrap();
        let s = build_system(dir.path());
        assert!(!s.contains("项目指令"));
        assert!(s.contains(&dir.path().display().to_string()));
    }

    #[test]
    fn agent_events_round_trip_serde_with_type_tag() {
        let sid = "s1".to_string();
        let evs = vec![
            AgentEvent::TextDelta {
                session_id: sid.clone(),
                delta: "hi".into(),
            },
            AgentEvent::ThinkingDelta {
                session_id: sid.clone(),
                delta: "hmm".into(),
            },
            AgentEvent::ToolStart {
                session_id: sid.clone(),
                tool_name: "bash".into(),
                args: serde_json::json!({"command": "ls"}),
            },
            AgentEvent::ToolProgress {
                session_id: sid.clone(),
                tool_name: "bash".into(),
                progress: "50%".into(),
            },
            AgentEvent::ToolEnd {
                session_id: sid.clone(),
                tool_name: "bash".into(),
                result: "out".into(),
            },
            AgentEvent::Finished {
                session_id: sid.clone(),
            },
            AgentEvent::Error {
                session_id: sid.clone(),
                message: "boom".into(),
                terminal: false,
            },
        ];
        for ev in &evs {
            let j = serde_json::to_string(ev).unwrap();
            let back: AgentEvent = serde_json::from_str(&j).unwrap();
            assert_eq!(&back, ev);
        }
        assert_eq!(
            serde_json::to_value(&AgentEvent::Finished {
                session_id: sid
            })
            .unwrap()["type"],
            "finished"
        );
    }

    /// 记录 system 提示词的 scripted client——验证「每个 turn 的首次模型请求
    /// 携带 AGENTS.md 内容」（spec：memory scenario 的请求侧断言）。
    struct RecordingClient {
        systems: Arc<std::sync::Mutex<Vec<String>>>,
        turns: std::sync::Mutex<VecDeque<LlmTurn>>,
    }

    #[async_trait]
    impl LlmClient for RecordingClient {
        async fn stream_turn(
            &self,
            req: &LlmRequest,
            _sink: &(dyn Fn(StreamEvent) + Send + Sync),
        ) -> Result<LlmTurn, LlmError> {
            self.systems.lock().unwrap().push(req.system.clone());
            self.turns
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| LlmError::terminal("recording script exhausted"))
        }
    }

    #[tokio::test]
    async fn every_turn_request_carries_memory_injection() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "AGENTS-MARKER-CONTENT")
            .unwrap();
        let systems = Arc::new(std::sync::Mutex::new(Vec::new()));
        let client = RecordingClient {
            systems: systems.clone(),
            turns: std::sync::Mutex::new(VecDeque::from(vec![
                FakeLlmClient::say("first"),
                FakeLlmClient::say("second"),
            ])),
        };
        let manager = SessionManager::new(
            Arc::new(client),
            ToolRegistry::new(Duration::from_secs(10)),
            SessionConfig::default(),
        );
        let handle = manager.create_session(dir.path().to_path_buf());
        let mut rx = handle.subscribe();
        handle.prompt("one").await;
        let _ = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
            .await
            .unwrap();
        handle.prompt("two").await;
        let _ = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
            .await
            .unwrap();

        let systems = systems.lock().unwrap();
        assert_eq!(systems.len(), 2);
        assert!(
            systems.iter().all(|s| s.contains("AGENTS-MARKER-CONTENT")),
            "every turn's request carries the memory injection"
        );
    }

    #[tokio::test]
    async fn two_sessions_never_cross_events_or_workdirs() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        // 有状态 fake：首轮按 prompt 文本写 marker 文件，次轮收尾。
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
        assert_ne!(h1.key, h2.key);
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

        // 事件不串台
        for ev in evs1.iter().chain(evs2.iter()) {
            let sid = serde_json::to_value(ev).unwrap()["session_id"]
                .as_str()
                .unwrap()
                .to_string();
            assert!(sid == h1.key || sid == h2.key);
        }
        assert!(evs1
            .iter()
            .all(|e| serde_json::to_value(e).unwrap()["session_id"] == h1.key));
        assert!(evs2
            .iter()
            .all(|e| serde_json::to_value(e).unwrap()["session_id"] == h2.key));

        // 工作目录互不可见
        let c1 = std::fs::read_to_string(dir1.path().join("marker.txt")).unwrap();
        let c2 = std::fs::read_to_string(dir2.path().join("marker.txt")).unwrap();
        assert_eq!(c1, "marker-one");
        assert_eq!(c2, "marker-two");
        assert!(!dir1.path().join("marker-two.txt").exists());
    }

    #[tokio::test]
    async fn session_survives_cancel_and_accepts_next_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![(
                "t1",
                "bash",
                serde_json::json!({"command": "sleep 30"}),
            )]),
            FakeLlmClient::say("second turn ran"),
        ]);
        let manager = SessionManager::new(
            Arc::new(llm),
            ToolRegistry::new(Duration::from_secs(60)),
            SessionConfig::default(),
        );
        let handle = manager.create_session(dir.path().to_path_buf());
        let mut rx = handle.subscribe();

        handle.prompt("long running").await;
        // 等 bash 启动再取消
        loop {
            let ev = rx.recv().await.unwrap();
            if matches!(ev, AgentEvent::ToolStart { .. }) {
                handle.cancel().await;
                break;
            }
        }
        let evs = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
            .await
            .unwrap();
        // 取消 → cancellation outcome，而非 finished；窗口里可能夹带
        // bash 的 ToolEnd（部分输出），但终态必须是取消 Error。
        assert!(
            !evs.iter().any(|e| matches!(e, AgentEvent::Finished { .. })),
            "cancel must not be a finished outcome: {evs:?}"
        );
        assert!(
            matches!(&evs[..], [.., AgentEvent::Error { message, terminal: false, .. }] if message == "turn cancelled"),
            "terminal event must be the cancellation outcome: {evs:?}"
        );

        // 同一会话继续可用：下一个 prompt 正常执行（C7）
        handle.prompt("again").await;
        let evs = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
            .await
            .unwrap();
        assert!(evs.iter().any(
            |e| matches!(e, AgentEvent::TextDelta { delta, .. } if delta == "second turn ran")
        ));
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::Finished { .. })));
    }

    #[tokio::test]
    async fn full_turn_event_sequence_over_session_stream() {
        let dir = tempfile::tempdir().unwrap();
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![(
                "t1",
                "write",
                serde_json::json!({"path": "a.txt", "content": "x"}),
            )]),
            FakeLlmClient::say("done"),
        ]);
        let manager = SessionManager::new(
            Arc::new(llm),
            ToolRegistry::new(Duration::from_secs(10)),
            SessionConfig::default(),
        );
        let handle = manager.create_session(dir.path().to_path_buf());
        let mut rx = handle.subscribe();
        handle.prompt("go").await;
        let evs = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
            .await
            .unwrap();

        let kinds: Vec<String> = evs
            .iter()
            .map(|e| serde_json::to_value(e).unwrap()["type"].as_str().unwrap().into())
            .collect();
        assert_eq!(
            kinds,
            vec!["tool_start", "tool_end", "tool_finish", "text_delta", "session_summary",
                 "finished"]
        );
        assert!(evs
            .iter()
            .all(|e| serde_json::to_value(e).unwrap()["session_id"] == handle.key));
    }

    #[tokio::test]
    async fn permission_answer_round_trip_through_session_handle() {
        use crate::policy::{ApproverHub, PolicyEngine};
        use crate::policy::ApprovalAnswer as AA;

        let dir = tempfile::tempdir().unwrap();
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![(
                "t1",
                "bash",
                serde_json::json!({"command": "rm -rf build"}),
            )]),
            FakeLlmClient::say("done"),
        ]);
        let manager = SessionManager::new(
            Arc::new(llm),
            ToolRegistry::new(Duration::from_secs(10)),
            SessionConfig::default(),
        )
        .with_policy(Arc::new(PolicyEngine::new(Default::default())))
        .with_approver(ApproverHub::new());
        let handle = manager.create_session(dir.path().to_path_buf());
        let mut rx = handle.subscribe();
        handle.prompt("go").await;

        // 收到审批请求 → 经 SessionHandle 回填 AllowOnce
        let mut request_id = String::new();
        loop {
            match rx.recv().await {
                Ok(AgentEvent::PermissionRequest { request_id: id, .. }) => {
                    request_id = id;
                    break;
                }
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("event channel closed: {e}"),
            }
        }
        handle.answer_permission(&request_id, AA::AllowOnce).await;

        let evs = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
            .await
            .unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, AgentEvent::ToolPolicy { outcome, .. } if outcome == "allowed_once")),
            "allow-once decision must round-trip: {evs:?}"
        );
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::Finished { .. })));
    }

    #[tokio::test]
    async fn prompts_queue_and_run_serially() {
        let dir = tempfile::tempdir().unwrap();
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::say("one"),
            FakeLlmClient::say("two"),
        ]);
        let manager = SessionManager::new(
            Arc::new(llm),
            ToolRegistry::new(Duration::from_secs(10)),
            SessionConfig::default(),
        );
        let handle = manager.create_session(dir.path().to_path_buf());
        let mut rx = handle.subscribe();
        // 两条 prompt 在第一 turn 仍在跑时提交
        handle.prompt("p1").await;
        handle.prompt("p2").await;

        let mut all = Vec::new();
        for _ in 0..2 {
            let evs = tokio::time::timeout(Duration::from_secs(30), wait_terminal(&mut rx))
                .await
                .unwrap();
            all.extend(evs);
        }
        let text: Vec<&String> = all
            .iter()
            .filter_map(|e| match e {
                AgentEvent::TextDelta { delta, .. } => Some(delta),
                _ => None,
            })
            .collect();
        assert_eq!(text, vec!["one", "two"], "serial order preserved");
        assert_eq!(
            all.iter().filter(|e| matches!(e, AgentEvent::Finished { .. })).count(),
            2
        );
    }
}
