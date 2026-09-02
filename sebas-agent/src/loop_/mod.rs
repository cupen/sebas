//! turn 循环引擎（task 4.1–4.3，design N7/N8）：状态机、三重预算、取消安全。
//!
//! 状态机（蓝图 §7）：Idle → AwaitingModel ⇄ ExecutingTools → Finished /
//! Cancelled / Failed。继续循环的判定以**内容**为准（响应含 tool_use 块）——
//! 与真实 Anthropic 流上的 `stop_reason == tool_use` 等价，同时让有状态
//! fake（恒返回 EndTurn）也能驱动多步测试。

use crate::llm::{LlmClient, LlmRequest, StopReason, StreamEvent};
use crate::message::{BudgetConfig, ContentBlock, Message, Role, ToolErrorKind, ToolOutput};
use crate::policy::{ApprovalAnswer, PermissionRequestInfo, PolicyDecision};
use crate::session::AgentEvent;
use crate::tools::{Tool, ToolCtx, ToolRegistry};
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

    /// 策略审批请求（request_id == tool_use_id，消费端据此回填决定）。
    pub(crate) fn permission_request(
        &self,
        request_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
        reason: &str,
    ) {
        self.send(AgentEvent::PermissionRequest {
            session_id: self.session_id.into(),
            request_id: request_id.into(),
            tool_name: tool_name.into(),
            args: args.clone(),
            reason: reason.into(),
        });
    }

    /// 策略流结果（allowed_once / allowed_session / escalated / denied / unavailable）。
    pub(crate) fn tool_policy(&self, request_id: &str, tool_name: &str, outcome: &str) {
        self.send(AgentEvent::ToolPolicy {
            session_id: self.session_id.into(),
            request_id: request_id.into(),
            tool_name: tool_name.into(),
            outcome: outcome.into(),
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
                    policy: tool_ctx_base.policy.clone(),
                    approver: tool_ctx_base.approver.clone(),
                };
                let out: ToolOutput = match registry.get(tool_name) {
                    Some(tool) => {
                        gated_execute(tool.as_ref(), tool_use_id, tool_name, input.clone(), &ctx, emit)
                            .await
                    }
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

/// 策略门控执行（task 1.3，design N1）：Allow 直跑；Deny 可升级（带理由的
/// 一次性重试）；Ask 走审批——无回答者/超时/取消 → fail closed。每次策略
/// 流结果发 `ToolPolicy` 事件，让消费端区分「策略拒绝」与「工具崩溃」。
async fn gated_execute(
    tool: &dyn Tool,
    tool_use_id: &str,
    tool_name: &str,
    input: serde_json::Value,
    ctx: &ToolCtx,
    emit: &TurnEmit<'_>,
) -> ToolOutput {
    let Some(policy) = ctx.policy.clone() else {
        // 未配置策略引擎：1a 行为（不过门控）。
        return tool.execute(input, ctx).await;
    };
    match policy.evaluate(tool_name, &input, &ctx.workdir) {
        PolicyDecision::Allow => tool.execute(input, ctx).await,
        PolicyDecision::Deny { reason } => {
            // 升级 = 带理由的一次性重试（仅当审批人放行那一次）；否则维持拒绝。
            if let Some(approver) = &ctx.approver {
                approver.prepare(tool_use_id);
                emit.permission_request(
                    tool_use_id,
                    tool_name,
                    &input,
                    &format!("denied by policy: {reason} — approve to run once?"),
                );
                let info = PermissionRequestInfo {
                    request_id: tool_use_id.to_string(),
                    tool: tool_name.to_string(),
                    input: input.clone(),
                    reason: reason.clone(),
                };
                let answer = wait_approval(approver.as_ref(), &info, &policy.config().approval_timeout, &ctx.cancel).await;
                if matches!(
                    answer,
                    Some(ApprovalAnswer::AllowOnce) | Some(ApprovalAnswer::Escalate { .. })
                ) {
                    emit.tool_policy(tool_use_id, tool_name, "escalated");
                    return tool.execute(input, ctx).await;
                }
            }
            emit.tool_policy(tool_use_id, tool_name, "denied");
            ToolOutput::error(
                ToolErrorKind::Denied { reason: reason.clone() },
                format!("refused by policy: {reason}"),
            )
        }
        PolicyDecision::Ask { reason } => {
            let Some(approver) = &ctx.approver else {
                // fail closed：无回答者 → 拒绝（结构化数据，循环继续）。
                emit.tool_policy(tool_use_id, tool_name, "unavailable");
                return ToolOutput::error(
                    ToolErrorKind::Denied { reason: reason.clone() },
                    format!(
                        "refused by policy: {reason}; no approval answerer reachable (fail closed)"
                    ),
                );
            };
            approver.prepare(tool_use_id);
            emit.permission_request(tool_use_id, tool_name, &input, &reason);
            let info = PermissionRequestInfo {
                request_id: tool_use_id.to_string(),
                tool: tool_name.to_string(),
                input: input.clone(),
                reason: reason.clone(),
            };
            let answer = wait_approval(approver.as_ref(), &info, &policy.config().approval_timeout, &ctx.cancel).await;
            match answer {
                Some(ApprovalAnswer::AllowOnce) => {
                    emit.tool_policy(tool_use_id, tool_name, "allowed_once");
                    tool.execute(input, ctx).await
                }
                Some(ApprovalAnswer::AllowSession) => {
                    // 本会话精确签名升级进 allowlist：后续同签名静默。
                    policy.allow_session(tool_name, &input);
                    emit.tool_policy(tool_use_id, tool_name, "allowed_session");
                    tool.execute(input, ctx).await
                }
                Some(ApprovalAnswer::Escalate { .. }) => {
                    emit.tool_policy(tool_use_id, tool_name, "escalated");
                    tool.execute(input, ctx).await
                }
                Some(ApprovalAnswer::Deny) => {
                    emit.tool_policy(tool_use_id, tool_name, "denied");
                    ToolOutput::error(
                        ToolErrorKind::Denied { reason: reason.clone() },
                        format!("refused by policy: operator denied `{tool_name}`"),
                    )
                }
                None => {
                    emit.tool_policy(tool_use_id, tool_name, "unavailable");
                    ToolOutput::error(
                        ToolErrorKind::Denied { reason: reason.clone() },
                        format!(
                            "refused by policy: {reason}; approval timed out or no answerer (fail closed)"
                        ),
                    )
                }
            }
        }
    }
}

/// 审批等待：与取消令牌竞争；超时（策略配置）或取消 → None（fail closed）。
async fn wait_approval(
    approver: &dyn crate::policy::Approver,
    info: &PermissionRequestInfo,
    timeout: &std::time::Duration,
    cancel: &tokio_util::sync::CancellationToken,
) -> Option<ApprovalAnswer> {
    tokio::select! {
        _ = cancel.cancelled() => None,
        a = tokio::time::timeout(*timeout, approver.approve(info)) => a.ok().flatten(),
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

    // ─── 策略门控（task 1.2/1.3 验证）─────────────────────────────────────

    struct ScriptedApprover(std::sync::Mutex<std::collections::VecDeque<ApprovalAnswer>>);

    fn registry_stub() -> crate::tools::ToolRegistry {
        crate::tools::ToolRegistry::new(std::time::Duration::from_secs(10))
    }

    use std::collections::VecDeque;
    use std::sync::Arc;

    #[async_trait::async_trait]
    impl crate::policy::Approver for ScriptedApprover {
        async fn approve(&self, _req: &PermissionRequestInfo) -> Option<ApprovalAnswer> {
            self.0.lock().unwrap().pop_front()
        }
    }

    fn gated_ctx(
        dir: &std::path::Path,
        policy: Arc<crate::policy::PolicyEngine>,
        approver: Option<Arc<dyn crate::policy::Approver>>,
    ) -> ToolCtx {
        ToolCtx {
            workdir: dir.to_path_buf(),
            cancel: tokio_util::sync::CancellationToken::new(),
            read_files: Default::default(),
            policy: Some(policy),
            approver,
        }
    }

    fn kinds(evs: &[AgentEvent]) -> Vec<String> {
        evs.iter()
            .map(|e| serde_json::to_value(e).unwrap()["type"].as_str().unwrap().into())
            .collect()
    }

    #[tokio::test]
    async fn allow_once_runs_once_then_asks_again() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::broadcast::channel(256);
        let policy = Arc::new(crate::policy::PolicyEngine::new(Default::default()));
        let approver: Arc<dyn crate::policy::Approver> = Arc::new(ScriptedApprover(std::sync::Mutex::new(VecDeque::from(vec![
            ApprovalAnswer::AllowOnce,
            ApprovalAnswer::Deny,
]))));
        let ctx = gated_ctx(dir.path(), policy, Some(approver));
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![("t1", "bash", serde_json::json!({"command": "rm -rf build"}))]),
            FakeLlmClient::call_tools(vec![("t2", "bash", serde_json::json!({"command": "rm -rf build"}))]),
            FakeLlmClient::say("done"),
        ]);
        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        let outcome = TurnEngine::new(BudgetConfig::default())
            .run_turn(&llm, &registry_stub(), &ctx, &mut history, "go", "sys", "m", tokio_util::sync::CancellationToken::new(), &emit)
            .await;
        assert_eq!(outcome, TurnOutcome::Finished { reason: FinishReason::EndTurn });
        let evs = collect(&mut rx);
        assert_eq!(
            kinds(&evs),
            vec!["tool_start", "permission_request", "tool_policy", "tool_end",
                 "tool_start", "permission_request", "tool_policy", "tool_end",
                 "text_delta"]
        );
        let outcomes: Vec<&str> = evs.iter().filter_map(|e| match e {
            AgentEvent::ToolPolicy { outcome, .. } => Some(outcome.as_str()),
            _ => None,
        }).collect();
        assert_eq!(outcomes, vec!["allowed_once", "denied"]);
        // 第二次被拒的结果是结构化数据回填（is_error），循环继续到 done
        assert!(matches!(
            &history[history.len() - 2].content[0],
            ContentBlock::ToolResult { is_error: true, .. }
        ));
    }

    #[tokio::test]
    async fn allow_session_absorbs_exact_signature() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::broadcast::channel(256);
        let policy = Arc::new(crate::policy::PolicyEngine::new(Default::default()));
        let approver: Arc<dyn crate::policy::Approver> = Arc::new(ScriptedApprover(std::sync::Mutex::new(VecDeque::from(vec![
            ApprovalAnswer::AllowSession,
]))));
        let ctx = gated_ctx(dir.path(), policy, Some(approver));
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![("t1", "bash", serde_json::json!({"command": "rm -rf build"}))]),
            FakeLlmClient::call_tools(vec![("t2", "bash", serde_json::json!({"command": "rm -rf build"}))]),
            FakeLlmClient::say("done"),
        ]);
        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        TurnEngine::new(BudgetConfig::default())
            .run_turn(&llm, &registry_stub(), &ctx, &mut history, "go", "sys", "m", tokio_util::sync::CancellationToken::new(), &emit)
            .await;
        let evs = collect(&mut rx);
        // 首次 ask → allowed_session；第二次同签名静默（无 permission_request / tool_policy）
        assert_eq!(
            kinds(&evs),
            vec!["tool_start", "permission_request", "tool_policy", "tool_end",
                 "tool_start", "tool_end", "text_delta"]
        );
    }

    #[tokio::test]
    async fn ask_without_approver_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::broadcast::channel(256);
        let policy = Arc::new(crate::policy::PolicyEngine::new(Default::default()));
        let ctx = gated_ctx(dir.path(), policy, None);
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![("t1", "bash", serde_json::json!({"command": "rm -rf build"}))]),
            FakeLlmClient::say("noted"),
        ]);
        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        let outcome = TurnEngine::new(BudgetConfig::default())
            .run_turn(&llm, &registry_stub(), &ctx, &mut history, "go", "sys", "m", tokio_util::sync::CancellationToken::new(), &emit)
            .await;
        assert_eq!(outcome, TurnOutcome::Finished { reason: FinishReason::EndTurn });
        let evs = collect(&mut rx);
        assert_eq!(
            kinds(&evs),
            vec!["tool_start", "tool_policy", "tool_end", "text_delta"]
        );
        assert!(matches!(&evs[1], AgentEvent::ToolPolicy { outcome, .. } if outcome == "unavailable"));
        assert!(matches!(
            &history[2].content[0],
            ContentBlock::ToolResult { is_error: true, content, .. } if content.contains("fail closed")
        ));
    }

    #[tokio::test]
    async fn static_deny_escalates_once_with_approval() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::broadcast::channel(256);
        let policy = Arc::new(crate::policy::PolicyEngine::new(crate::policy::PolicyConfig {
            deny: vec![crate::policy::ToolRule::tool("bash")],
            ..Default::default()
        }));
        let approver: Arc<dyn crate::policy::Approver> = Arc::new(ScriptedApprover(std::sync::Mutex::new(VecDeque::from(vec![
            ApprovalAnswer::AllowOnce,
]))));
        let ctx = gated_ctx(dir.path(), policy, Some(approver));
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![("t1", "bash", serde_json::json!({"command": "echo escalated-run"}))]),
            FakeLlmClient::say("done"),
        ]);
        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        TurnEngine::new(BudgetConfig::default())
            .run_turn(&llm, &registry_stub(), &ctx, &mut history, "go", "sys", "m", tokio_util::sync::CancellationToken::new(), &emit)
            .await;
        let evs = collect(&mut rx);
        assert_eq!(
            kinds(&evs),
            vec!["tool_start", "permission_request", "tool_policy", "tool_end", "text_delta"]
        );
        assert!(matches!(&evs[2], AgentEvent::ToolPolicy { outcome, .. } if outcome == "escalated"));
        // 升级确实执行了（输出可见），且只此一次
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::ToolEnd { result, .. } if result.contains("escalated-run"))));
    }

    #[tokio::test]
    async fn no_policy_engine_keeps_1a_behavior() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::broadcast::channel(256);
        let ctx = gated_ctx(dir.path(), Arc::new(crate::policy::PolicyEngine::new(Default::default())), None);
        // policy=None 场景由既有测试覆盖（SessionManager 默认）；这里验证 rm 命令
        // 在 policy=None 时直接执行——用一个显式 None ctx 变体。
        let ctx_none = ToolCtx {
            policy: None,
            approver: None,
            ..ctx
        };
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![("t1", "bash", serde_json::json!({"command": "echo raw-bash"}))]),
            FakeLlmClient::say("done"),
        ]);
        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        TurnEngine::new(BudgetConfig::default())
            .run_turn(&llm, &registry_stub(), &ctx_none, &mut history, "go", "sys", "m", tokio_util::sync::CancellationToken::new(), &emit)
            .await;
        let evs = collect(&mut rx);
        assert!(!evs.iter().any(|e| matches!(e, AgentEvent::PermissionRequest { .. } | AgentEvent::ToolPolicy { .. })));
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::ToolEnd { result, .. } if result.contains("raw-bash"))));
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
