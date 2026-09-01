//! turn 循环引擎（task 4.1–4.3，design N7/N8）：状态机、三重预算、取消安全。
//!
//! 状态机（蓝图 §7）：Idle → AwaitingModel ⇄ ExecutingTools → Finished /
//! Cancelled / Failed。继续循环的判定以**内容**为准（响应含 tool_use 块）——
//! 与真实 Anthropic 流上的 `stop_reason == tool_use` 等价，同时让有状态
//! fake（恒返回 EndTurn）也能驱动多步测试。

use crate::llm::{LlmClient, LlmRequest, StopReason, StreamEvent};
use crate::message::{BudgetConfig, ContentBlock, Message, Role, ToolErrorKind, ToolOutput};
use crate::session::AgentEvent;
use crate::tools::{ToolCtx, ToolRegistry};
use tokio_util::sync::CancellationToken;

/// 正常收尾原因。预算耗尽也走 Finished（spec：不是 error）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    EndTurn,
    MaxTokens,
    /// `which`: "model_calls" | "tool_calls" | "turn_deadline"
    Budget {
        which: &'static str,
    },
}

/// 一轮 turn 的结局。终态事件（Finished / Error）由会话层根据结局统一发射。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    Finished {
        reason: FinishReason,
    },
    Cancelled,
    Failed {
        terminal: bool,
        message: String,
    },
}

/// 事件发射器：引擎内部事件 → 会话级 AgentEvent（带 session_id）。
/// 类型必须可命名（出现在 pub fn 签名中）；构造与发射方法为 crate 内部。
pub struct TurnEmit<'a> {
    session_id: &'a str,
    tx: &'a tokio::sync::broadcast::Sender<AgentEvent>,
}

impl<'a> TurnEmit<'a> {
    pub(crate) fn new(session_id: &'a str, tx: &'a tokio::sync::broadcast::Sender<AgentEvent>) -> Self {
        Self { session_id, tx }
    }

    fn send(&self, ev: AgentEvent) {
        // 无订阅者（headless）时 broadcast 返回 Err——正常，忽略。
        let _ = self.tx.send(ev);
    }

    pub(crate) fn text_delta(&self, delta: &str) {
        self.send(AgentEvent::TextDelta {
            session_id: self.session_id.into(),
            delta: delta.into(),
        });
    }

    #[allow(dead_code)]
    pub(crate) fn thinking_delta(&self, delta: &str) {
        self.send(AgentEvent::ThinkingDelta {
            session_id: self.session_id.into(),
            delta: delta.into(),
        });
    }

    pub(crate) fn tool_start(&self, tool_name: &str, args: serde_json::Value) {
        self.send(AgentEvent::ToolStart {
            session_id: self.session_id.into(),
            tool_name: tool_name.into(),
            args,
        });
    }

    #[allow(dead_code)]
    pub(crate) fn tool_progress(&self, tool_name: &str, progress: &str) {
        self.send(AgentEvent::ToolProgress {
            session_id: self.session_id.into(),
            tool_name: tool_name.into(),
            progress: progress.into(),
        });
    }

    pub(crate) fn tool_end(&self, tool_name: &str, result: &str) {
        self.send(AgentEvent::ToolEnd {
            session_id: self.session_id.into(),
            tool_name: tool_name.into(),
            result: result.into(),
        });
    }
}

/// turn 引擎：只持有预算配置；会话历史由调用方持有并传入（每会话一份）。
pub struct TurnEngine {
    pub budget: BudgetConfig,
}

impl TurnEngine {
    pub fn new(budget: BudgetConfig) -> Self {
        Self { budget }
    }

    /// 执行一轮 turn：把 `user_text` 追加进 `history`，循环「模型 ⇄ 工具」
    /// 直到无工具调用 / 预算耗尽 / 失败 / 取消。
    #[allow(clippy::too_many_arguments)]
    pub async fn run_turn(
        &self,
        llm: &dyn LlmClient,
        registry: &ToolRegistry,
        tool_ctx_base: &ToolCtx,
        history: &mut Vec<Message>,
        user_text: &str,
        system: &str,
        model: &str,
        cancel: CancellationToken,
        emit: &TurnEmit<'_>,
    ) -> TurnOutcome {
        history.push(Message::user_text(user_text));
        let deadline = tokio::time::Instant::now() + self.budget.turn_timeout;
        let mut model_calls: u32 = 0;
        let mut tool_calls: u32 = 0;

        loop {
            if cancel.is_cancelled() {
                return TurnOutcome::Cancelled;
            }
            if model_calls >= self.budget.max_model_calls {
                return TurnOutcome::Finished {
                    reason: FinishReason::Budget {
                        which: "model_calls",
                    },
                };
            }
            // 时限检查放在 select 之外：select 对多个就绪分支随机选择，
            // 即时的模型调用可能与已到期的 deadline 竞争，导致多打一次模型。
            if tokio::time::Instant::now() >= deadline {
                return TurnOutcome::Finished {
                    reason: FinishReason::Budget {
                        which: "turn_deadline",
                    },
                };
            }
            model_calls += 1;

            // AwaitingModel：delta 到达即回调；select 保证取消/超时即时生效。
            let req = LlmRequest {
                model: model.to_string(),
                system: system.to_string(),
                messages: history.clone(),
                tools: registry.schemas(),
                max_tokens: 8192,
            };
            let sink = |ev: StreamEvent| match ev {
                StreamEvent::TextDelta(d) => emit.text_delta(&d),
                StreamEvent::ThinkingDelta(d) => emit.thinking_delta(&d),
            };
            let turn = tokio::select! {
                _ = cancel.cancelled() => return TurnOutcome::Cancelled,
                _ = tokio::time::sleep_until(deadline) => {
                    return TurnOutcome::Finished {
                        reason: FinishReason::Budget { which: "turn_deadline" },
                    };
                }
                r = llm.stream_turn(&req, &sink) => match r {
                    Ok(t) => t,
                    Err(e) => {
                        return TurnOutcome::Failed { terminal: e.terminal, message: e.message };
                    }
                },
            };

            history.push(Message {
                role: Role::Assistant,
                content: turn.content.clone(),
            });

            let has_tool_use = turn
                .content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
            if turn.stop_reason == StopReason::MaxTokens || !has_tool_use {
                return TurnOutcome::Finished {
                    reason: if turn.stop_reason == StopReason::MaxTokens {
                        FinishReason::MaxTokens
                    } else {
                        FinishReason::EndTurn
                    },
                };
            }

            // ExecutingTools：按序执行全部 tool_use（Phase 1 串行）。
            for block in &turn.content {
                let (tool_use_id, tool_name, input) = match block {
                    ContentBlock::ToolUse { id, name, input } => (id, name, input),
                    _ => continue,
                };
                if cancel.is_cancelled() {
                    return TurnOutcome::Cancelled;
                }
                // 墙钟预算在工具间同样生效（deadline 过 → 不再执行下一个工具）。
                if tokio::time::Instant::now() >= deadline {
                    return TurnOutcome::Finished {
                        reason: FinishReason::Budget { which: "turn_deadline" },
                    };
                }
                if tool_calls >= self.budget.max_tool_calls {
                    return TurnOutcome::Finished {
                        reason: FinishReason::Budget { which: "tool_calls" },
                    };
                }
                tool_calls += 1;
                emit.tool_start(tool_name, input.clone());

                // 每 turn 换发取消令牌，read_files 集合随会话共享。
                let ctx = ToolCtx {
                    workdir: tool_ctx_base.workdir.clone(),
                    cancel: cancel.clone(),
                    read_files: tool_ctx_base.read_files.clone(),
                };
                let out: ToolOutput = match registry.get(tool_name) {
                    Some(tool) => tool.execute(input.clone(), &ctx).await,
                    None => ToolOutput::error(
                        ToolErrorKind::InvalidArgs,
                        format!(
                            "unknown tool `{tool_name}`; available: {}",
                            registry.names().join(", ")
                        ),
                    ),
                };
                // 错误是数据（C4）：ToolEnd 的文本必须让模型看见失败原因。
                let end_text = match &out.error {
                    Some(kind) => format!("{}: {}", kind, out.output),
                    None => out.output.clone(),
                };
                emit.tool_end(tool_name, &end_text);
                // 工具结果回填为 user/tool_result（错误是数据，C4）。
                history.push(Message {
                    role: Role::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: out.output.clone(),
                        is_error: !out.ok,
                    }],
                });
                if cancel.is_cancelled() {
                    return TurnOutcome::Cancelled;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::fake::FakeLlmClient;
    use std::time::Duration;

    fn setup(dir: &std::path::Path) -> (ToolCtx, tokio::sync::broadcast::Sender<AgentEvent>, tokio::sync::broadcast::Receiver<AgentEvent>) {
        let (tx, rx) = tokio::sync::broadcast::channel(256);
        (
            ToolCtx::new(dir.to_path_buf(), CancellationToken::new()),
            tx,
            rx,
        )
    }

    fn collect(rx: &mut tokio::sync::broadcast::Receiver<AgentEvent>) -> Vec<AgentEvent> {
        let mut evs = Vec::new();
        while let Ok(e) = rx.try_recv() {
            evs.push(e);
        }
        evs
    }

    #[tokio::test]
    async fn multi_step_five_tools_finish_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tx, mut rx) = setup(dir.path());
        let registry = ToolRegistry::new(Duration::from_secs(10));
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![(
                "t1",
                "write",
                serde_json::json!({"path": "a.txt", "content": "hello world"}),
            )]),
            FakeLlmClient::call_tools(vec![(
                "t2",
                "read",
                serde_json::json!({"path": "a.txt"}),
            )]),
            FakeLlmClient::call_tools(vec![(
                "t3",
                "edit",
                serde_json::json!({"path": "a.txt", "old_string": "world", "new_string": "there"}),
            )]),
            FakeLlmClient::call_tools(vec![(
                "t4",
                "grep",
                serde_json::json!({"pattern": "there"}),
            )]),
            FakeLlmClient::call_tools(vec![(
                "t5",
                "glob",
                serde_json::json!({"pattern": "*.txt"}),
            )]),
            FakeLlmClient::say("all done"),
        ]);

        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        let outcome = TurnEngine::new(BudgetConfig::default())
            .run_turn(
                &llm,
                &registry,
                &ctx,
                &mut history,
                "do the thing",
                "sys",
                "m",
                CancellationToken::new(),
                &emit,
            )
            .await;

        assert_eq!(
            outcome,
            TurnOutcome::Finished {
                reason: FinishReason::EndTurn
            }
        );
        // 文件经 5 步工具链演进到位
        let content = std::fs::read_to_string(dir.path().join("a.txt")).unwrap();
        assert_eq!(content, "hello there");
        // 1 user + 6 assistant + 5 tool_result；最后一条是 assistant 文本
        assert_eq!(history.len(), 12);
        assert!(matches!(
            history.last().unwrap().content[0],
            ContentBlock::Text { .. }
        ));

        // 事件序：每工具 start→end；终态事件属于会话层，引擎不发
        let evs = collect(&mut rx);
        let starts: Vec<&str> = evs
            .iter()
            .filter_map(|e| match e {
                AgentEvent::ToolStart { tool_name, .. } => Some(tool_name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(starts, vec!["write", "read", "edit", "grep", "glob"]);
        assert_eq!(
            evs.iter()
                .filter(|e| matches!(e, AgentEvent::ToolEnd { .. }))
                .count(),
            5
        );
        assert!(evs.iter().any(|e| matches!(
            e,
            AgentEvent::TextDelta { delta, .. } if delta == "all done"
        )));
        assert!(
            !evs.iter()
                .any(|e| matches!(e, AgentEvent::Finished { .. })),
            "terminal event belongs to the session layer"
        );
        // 所有事件都带同一 session_id
        assert!(evs
            .iter()
            .all(|e| serde_json::to_value(e).unwrap()["session_id"] == "s1"));
    }

    #[tokio::test]
    async fn model_call_budget_ends_as_finished_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tx, mut rx) = setup(dir.path());
        let registry = ToolRegistry::new(Duration::from_secs(10));
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
        let budget = BudgetConfig {
            max_model_calls: 2,
            ..Default::default()
        };

        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        let outcome = TurnEngine::new(budget)
            .run_turn(
                &llm, &registry, &ctx, &mut history, "go", "sys", "m",
                CancellationToken::new(), &emit,
            )
            .await;

        assert_eq!(
            outcome,
            TurnOutcome::Finished {
                reason: FinishReason::Budget {
                    which: "model_calls"
                }
            }
        );
        // 两轮工具都执行并回填
        assert_eq!(history.len(), 5);
        let evs = collect(&mut rx);
        assert!(!evs.iter().any(|e| matches!(e, AgentEvent::Error { .. })));
    }

    #[tokio::test]
    async fn tool_call_budget_ends_as_finished_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tx, mut rx) = setup(dir.path());
        let registry = ToolRegistry::new(Duration::from_secs(10));
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
            FakeLlmClient::call_tools(vec![(
                "t3",
                "bash",
                serde_json::json!({"command": "echo three"}),
            )]),
        ]);
        let budget = BudgetConfig {
            max_tool_calls: 2,
            ..Default::default()
        };

        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        let outcome = TurnEngine::new(budget)
            .run_turn(
                &llm, &registry, &ctx, &mut history, "go", "sys", "m",
                CancellationToken::new(), &emit,
            )
            .await;

        assert_eq!(
            outcome,
            TurnOutcome::Finished {
                reason: FinishReason::Budget { which: "tool_calls" }
            }
        );
        // 第三个工具调用未执行
        let evs = collect(&mut rx);
        assert_eq!(
            evs.iter()
                .filter(|e| matches!(e, AgentEvent::ToolEnd { .. }))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn deadline_budget_ends_as_finished_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tx, _rx) = setup(dir.path());
        let registry = ToolRegistry::new(Duration::from_secs(10));
        let llm = FakeLlmClient::scripted(vec![FakeLlmClient::call_tools(vec![(
            "t1",
            "bash",
            serde_json::json!({"command": "sleep 1"}),
        )])]);
        let budget = BudgetConfig {
            turn_timeout: Duration::from_millis(150),
            ..Default::default()
        };

        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        let outcome = TurnEngine::new(budget)
            .run_turn(
                &llm, &registry, &ctx, &mut history, "go", "sys", "m",
                CancellationToken::new(), &emit,
            )
            .await;

        assert_eq!(
            outcome,
            TurnOutcome::Finished {
                reason: FinishReason::Budget {
                    which: "turn_deadline"
                }
            }
        );
    }

    #[tokio::test]
    async fn cancel_mid_bash_preserves_history_and_session_reusable() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tx, mut rx) = setup(dir.path());
        let registry = ToolRegistry::new(Duration::from_secs(60));
        let llm = FakeLlmClient::scripted(vec![FakeLlmClient::call_tools(vec![(
            "t1",
            "bash",
            serde_json::json!({"command": "echo begun; sleep 30"}),
        )])]);

        let cancel = CancellationToken::new();
        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        let engine = TurnEngine::new(BudgetConfig::default());
        let started = std::time::Instant::now();
        let outcome = {
            // fut 的借用全部收在块作用域内，出块后 history 才能再被读/借。
            let fut = engine.run_turn(
                &llm,
                &registry,
                &ctx,
                &mut history,
                "go",
                "sys",
                "m",
                cancel.clone(),
                &emit,
            );
            tokio::pin!(fut);
            // 等 bash 启动（ToolStart 事件）再取消
            loop {
                tokio::select! {
                    ev = rx.recv() => {
                        if matches!(ev, Ok(AgentEvent::ToolStart { .. })) {
                            cancel.cancel();
                        }
                    }
                    out = &mut fut => break out,
                }
            }
        };

        assert_eq!(outcome, TurnOutcome::Cancelled);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "sleep 30 must be killed promptly"
        );
        // 历史保留：user + assistant(tool_use) + tool_result(取消)
        assert_eq!(history.len(), 3);
        assert!(matches!(
            &history[2].content[0],
            ContentBlock::ToolResult { is_error: true, .. }
        ));

        // 同一 engine / history / ctx 再跑一轮 → 正常完成（C7 会话可复用）
        let llm2 = FakeLlmClient::scripted(vec![FakeLlmClient::say("fresh turn")]);
        let outcome2 = engine
            .run_turn(
                &llm2, &registry, &ctx, &mut history, "again", "sys", "m",
                CancellationToken::new(), &emit,
            )
            .await;
        assert_eq!(
            outcome2,
            TurnOutcome::Finished {
                reason: FinishReason::EndTurn
            }
        );
        // 3（取消轮）+ user("again") + assistant(text) = 5
        assert_eq!(history.len(), 5);
    }

    #[tokio::test]
    async fn unknown_tool_is_error_data_not_crash() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tx, _rx) = setup(dir.path());
        let registry = ToolRegistry::new(Duration::from_secs(10));
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![("t1", "nonexistent", serde_json::json!({}))]),
            FakeLlmClient::say("ok, noted"),
        ]);

        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        let outcome = TurnEngine::new(BudgetConfig::default())
            .run_turn(
                &llm, &registry, &ctx, &mut history, "go", "sys", "m",
                CancellationToken::new(), &emit,
            )
            .await;

        assert_eq!(
            outcome,
            TurnOutcome::Finished {
                reason: FinishReason::EndTurn
            }
        );
        // 未知工具 → 结构化错误结果回填，循环继续
        assert!(matches!(
            &history[2].content[0],
            ContentBlock::ToolResult { content, is_error: true, .. }
                if content.contains("unknown tool")
        ));
    }
}
