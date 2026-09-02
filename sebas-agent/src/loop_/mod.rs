//! turn 循环引擎（task 4.1–4.3，design N7/N8）：状态机、三重预算、取消安全。
//!
//! 状态机（蓝图 §7）：Idle → AwaitingModel ⇄ ExecutingTools → Finished /
//! Cancelled / Failed。继续循环的判定以**内容**为准（响应含 tool_use 块）——
//! 与真实 Anthropic 流上的 `stop_reason == tool_use` 等价，同时让有状态
//! fake（恒返回 EndTurn）也能驱动多步测试。

use crate::llm::{LlmClient, LlmRequest, StopReason, StreamEvent};
use crate::message::{
    rewrite_for_history, BudgetConfig, ContentBlock, Message, Role, ToolErrorKind, ToolOutput,
};
use crate::policy::{ApprovalAnswer, PermissionRequestInfo, PolicyDecision};
use crate::session::AgentEvent;
use crate::tools::{Tool, ToolCtx, ToolRegistry};
use tokio_util::sync::CancellationToken;

/// 正常收尾原因。预算耗尽也走 Finished（spec：不是 error）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    EndTurn,
    MaxTokens,
    /// `which`: "model_calls" | "tool_calls" | "turn_deadline" | "messages" | "tokens"
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

    /// 工具结构化收尾（ToolEnd 的孪生事件，design N6）。
    pub(crate) fn tool_finish(&self, tool_name: &str, out: &ToolOutput) {
        self.send(AgentEvent::ToolFinish {
            session_id: self.session_id.into(),
            tool_name: tool_name.into(),
            ok: out.ok,
            truncated: out.truncated,
            exit_code: out.exit_code,
        });
    }

    /// turn 汇总（引擎收尾发射，design N6）。
    pub(crate) fn session_summary(&self, model_calls: u32, tool_calls: u32, turn_ms: u64) {
        self.send(AgentEvent::SessionSummary {
            session_id: self.session_id.into(),
            model_calls,
            tool_calls,
            turn_ms,
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

/// turn 内计数器（design N6 SessionSummary 的数据源）。
#[derive(Default)]
struct TurnCounters {
    model_calls: u32,
    tool_calls: u32,
}

/// turn 引擎：持有预算与并发配置；会话历史由调用方持有并传入（每会话一份）。
pub struct TurnEngine {
    pub budget: BudgetConfig,
    pub max_concurrent_readonly: usize,
}

impl TurnEngine {
    pub fn new(budget: BudgetConfig) -> Self {
        Self {
            budget,
            max_concurrent_readonly: 8,
        }
    }

    /// 执行一轮 turn（公开入口）：计时并在收尾发射 `SessionSummary`。
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
        let started = std::time::Instant::now();
        let mut counters = TurnCounters::default();
        let outcome = self
            .run_turn_inner(llm, registry, tool_ctx_base, history, user_text, system, model, cancel, emit, &mut counters)
            .await;
        emit.session_summary(
            counters.model_calls,
            counters.tool_calls,
            started.elapsed().as_millis() as u64,
        );
        outcome
    }

    /// 执行一轮 turn：把 `user_text` 追加进 `history`，循环「模型 ⇄ 工具」
    /// 直到无工具调用 / 预算耗尽 / 失败 / 取消。
    #[allow(clippy::too_many_arguments)]
    async fn run_turn_inner(
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
        counters: &mut TurnCounters,
    ) -> TurnOutcome {
        history.push(Message::user_text(user_text));
        let deadline = tokio::time::Instant::now() + self.budget.turn_timeout;

        loop {
            if cancel.is_cancelled() {
                return TurnOutcome::Cancelled;
            }
            if counters.model_calls >= self.budget.max_model_calls {
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
            // Assembly 预算（task 3.2，C8 token 维度）：超限 = 干净收尾（非错误）。
            if history.len() > self.budget.max_messages {
                return TurnOutcome::Finished {
                    reason: FinishReason::Budget { which: "messages" },
                };
            }
            if estimate_tokens(system, history) > self.budget.est_token_budget {
                return TurnOutcome::Finished {
                    reason: FinishReason::Budget { which: "tokens" },
                };
            }
            counters.model_calls += 1;

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

            // ExecutingTools：按响应序执行 tool_use——连续只读段并行
            //（task 3.3，cap 限流），写/未知工具与相邻段保持先后。
            let calls: Vec<(&str, &str, &serde_json::Value)> = turn
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, name, input } => Some((id.as_str(), name.as_str(), input)),
                    _ => None,
                })
                .collect();
            let mut outputs: Vec<Option<ToolOutput>> = vec![None; calls.len()];
            let mut i = 0usize;
            let mut budget_exhausted: Option<&'static str> = None;
            while i < calls.len() {
                if cancel.is_cancelled() {
                    return TurnOutcome::Cancelled;
                }
                // 墙钟预算在工具间同样生效。
                if tokio::time::Instant::now() >= deadline {
                    budget_exhausted = Some("turn_deadline");
                    break;
                }
                let readonly = is_readonly_tool(calls[i].1);
                if readonly {
                    let mut j = i;
                    while j < calls.len() && is_readonly_tool(calls[j].1) {
                        j += 1;
                    }
                    // 预算裁剪：段内调用逐个计数，超额部分不执行（预算收尾）。
                    let remaining = (self.budget.max_tool_calls.saturating_sub(counters.tool_calls)) as usize;
                    let run_end = i + remaining.min(j - i);
                    if run_end < j {
                        budget_exhausted = Some("tool_calls");
                    }
                    // 段内按 cap 分批：每批 emit starts（响应序）→ 并行执行 → ends（响应序）。
                    let mut batch = i;
                    while batch < run_end {
                        let batch_end = (batch + self.max_concurrent_readonly).min(run_end);
                        for (_id, name, input) in &calls[batch..batch_end] {
                            emit.tool_start(name, (*input).clone());
                        }
                        let ctx = make_ctx(tool_ctx_base, &cancel);
                        let futs = calls[batch..batch_end].iter().map(|(id, name, input)| {
                            execute_one(registry, id, name, input, &ctx, emit)
                        });
                        let done = futures_util::future::join_all(futs).await;
                        for (k, out) in done.into_iter().enumerate() {
                            emit_tool_end(emit, calls[batch + k].1, &out);
                            outputs[batch + k] = Some(out);
                            counters.tool_calls += 1;
                        }
                        batch = batch_end;
                    }
                    i = j;
                } else {
                    if counters.tool_calls >= self.budget.max_tool_calls {
                        budget_exhausted = Some("tool_calls");
                        break;
                    }
                    let (id, name, input) = calls[i];
                    emit.tool_start(name, input.clone());
                    let ctx = make_ctx(tool_ctx_base, &cancel);
                    let out = execute_one(registry, id, name, input, &ctx, emit).await;
                    emit_tool_end(emit, name, &out);
                    outputs[i] = Some(out);
                    counters.tool_calls += 1;
                    i += 1;
                }
            }
            // tool_result 按响应序回填（入库副本经 3.1 改写）。
            for (k, (id, _, _)) in calls.iter().enumerate() {
                if let Some(out) = &outputs[k] {
                    history.push(Message {
                        role: Role::User,
                        content: vec![ContentBlock::ToolResult {
                            tool_use_id: (*id).to_string(),
                            content: rewrite_for_history(&out.output),
                            is_error: !out.ok,
                        }],
                    });
                }
            }
            if cancel.is_cancelled() {
                return TurnOutcome::Cancelled;
            }
            if let Some(which) = budget_exhausted {
                return TurnOutcome::Finished {
                    reason: FinishReason::Budget { which },
                };
            }
        }
    }
}

/// 粗粒度 token 估算（task 3.2）：chars/4 + 每块常数开销（不追求精确，
/// 只求比真值略高以保守收尾）。
fn estimate_tokens(system: &str, history: &[Message]) -> usize {
    let mut tokens = system.chars().count() / 4;
    for m in history {
        for b in &m.content {
            tokens += 8;
            let chars = match b {
                ContentBlock::Text { text } => text.chars().count(),
                ContentBlock::ToolUse { input, .. } => input.to_string().chars().count(),
                ContentBlock::ToolResult { content, .. } => content.chars().count(),
                ContentBlock::Thinking { .. } | ContentBlock::Image { .. } => 0,
            };
            tokens += chars / 4;
        }
    }
    tokens
}

/// 只读工具（task 3.3 并行白名单）——名称即契约；未知工具按写处理（保守串行）。
fn is_readonly_tool(name: &str) -> bool {
    matches!(name, "read" | "glob" | "grep" | "web_search" | "web_fetch" | "read_image" | "lsp")
}

fn make_ctx(base: &ToolCtx, cancel: &CancellationToken) -> ToolCtx {
    ToolCtx {
        workdir: base.workdir.clone(),
        cancel: cancel.clone(),
        read_files: base.read_files.clone(),
        policy: base.policy.clone(),
        approver: base.approver.clone(),
    }
}

/// 单次执行：gated（策略门控）+ 未知工具兜底。
async fn execute_one(
    registry: &ToolRegistry,
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    ctx: &ToolCtx,
    emit: &TurnEmit<'_>,
) -> ToolOutput {
    match registry.get(tool_name) {
        Some(tool) => {
            gated_execute(tool.as_ref(), tool_use_id, tool_name, input.clone(), ctx, emit).await
        }
        None => ToolOutput::error(
            ToolErrorKind::InvalidArgs,
            format!(
                "unknown tool `{tool_name}`; available: {}",
                registry.names().join(", ")
            ),
        ),
    }
}

/// ToolEnd 文本：错误是数据（C4），失败原因必须让模型看见。
/// 同时发射结构化孪生事件 ToolFinish（design N6）。
fn emit_tool_end(emit: &TurnEmit<'_>, tool_name: &str, out: &ToolOutput) {
    let end_text = match &out.error {
        Some(kind) => format!("{}: {}", kind, out.output),
        None => out.output.clone(),
    };
    emit.tool_end(tool_name, &end_text);
    emit.tool_finish(tool_name, out);
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
            vec!["tool_start", "permission_request", "tool_policy", "tool_end", "tool_finish",
                 "tool_start", "permission_request", "tool_policy", "tool_end", "tool_finish",
                 "text_delta", "session_summary"]
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
            vec!["tool_start", "permission_request", "tool_policy", "tool_end", "tool_finish",
                 "tool_start", "tool_end", "tool_finish", "text_delta", "session_summary"]
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
            vec!["tool_start", "tool_policy", "tool_end", "tool_finish", "text_delta",
                 "session_summary"]
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
            vec!["tool_start", "permission_request", "tool_policy", "tool_end", "tool_finish",
                 "text_delta", "session_summary"]
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

    // ─── 上下文管理（task 3.1–3.3 验证）──────────────────────────────────

    struct SlowRead {
        concurrent: Arc<std::sync::atomic::AtomicUsize>,
        peak: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl crate::tools::Tool for SlowRead {
        fn name(&self) -> &'static str {
            "read"
        }
        fn description(&self) -> String {
            "slow read stub for concurrency testing".into()
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolCtx) -> ToolOutput {
            use std::sync::atomic::Ordering;
            let now = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            self.concurrent.fetch_sub(1, Ordering::SeqCst);
            ToolOutput::ok("slow-ok")
        }
    }

    #[tokio::test]
    async fn large_tool_result_is_rewritten_in_history_only() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tx, mut rx) = setup(dir.path());
        let registry = ToolRegistry::new(std::time::Duration::from_secs(10));
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![(
                "t1",
                "bash",
                serde_json::json!({"command": "yes word | head -c 20000"}),
            )]),
            FakeLlmClient::say("done"),
        ]);
        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        TurnEngine::new(BudgetConfig::default())
            .run_turn(&llm, &registry, &ctx, &mut history, "go", "sys", "m", CancellationToken::new(), &emit)
            .await;
        // 入库副本：改写标记 + 长度有界
        let stored = match &history[2].content[0] {
            ContentBlock::ToolResult { content, .. } => content,
            other => panic!("expected tool_result, got {other:?}"),
        };
        assert!(stored.contains("[truncated:"), "{}", &stored[..200]);
        assert!(stored.chars().count() < 8_300, "stored must be bounded");
        // 事件面：ToolEnd 保留改写前（cap 后）版本，无入库标记
        let evs = collect(&mut rx);
        let end_full = evs.iter().find_map(|e| match e {
            AgentEvent::ToolEnd { result, .. } => Some(result.clone()),
            _ => None,
        }).unwrap();
        assert!(end_full.chars().count() > 15_000, "ToolEnd keeps pre-rewrite text");
        assert!(!end_full.contains("[truncated:"));
    }

    #[test]
    fn schemas_carry_rewrite_note() {
        let registry = ToolRegistry::new(std::time::Duration::from_secs(10));
        for s in registry.schemas() {
            assert!(s.description.contains("[truncated"), "{} description must mention rewriting", s.name);
        }
    }

    #[tokio::test]
    async fn message_budget_ends_turn_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tx, _rx) = setup(dir.path());
        let registry = ToolRegistry::new(std::time::Duration::from_secs(10));
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![("t1", "bash", serde_json::json!({"command": "echo one"}))]),
            FakeLlmClient::call_tools(vec![("t2", "bash", serde_json::json!({"command": "echo two"}))]),
            FakeLlmClient::say("never reached"),
        ]);
        let budget = BudgetConfig { max_messages: 3, ..Default::default() };
        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        let outcome = TurnEngine::new(budget)
            .run_turn(&llm, &registry, &ctx, &mut history, "go", "sys", "m", CancellationToken::new(), &emit)
            .await;
        assert_eq!(
            outcome,
            TurnOutcome::Finished { reason: FinishReason::Budget { which: "messages" } }
        );
    }

    #[tokio::test]
    async fn token_budget_ends_turn_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tx, _rx) = setup(dir.path());
        let registry = ToolRegistry::new(std::time::Duration::from_secs(10));
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![(
                "t1",
                "bash",
                serde_json::json!({"command": "yes word | head -c 2000"}),
            )]),
            FakeLlmClient::say("never reached"),
        ]);
        let budget = BudgetConfig { est_token_budget: 100, ..Default::default() };
        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        let outcome = TurnEngine::new(budget)
            .run_turn(&llm, &registry, &ctx, &mut history, "go", "sys", "m", CancellationToken::new(), &emit)
            .await;
        assert_eq!(
            outcome,
            TurnOutcome::Finished { reason: FinishReason::Budget { which: "tokens" } }
        );
    }

    #[tokio::test]
    async fn consecutive_readonly_calls_run_concurrently_in_response_order() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tx, mut rx) = setup(dir.path());
        let concurrent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let registry = ToolRegistry::from_tools(vec![Arc::new(SlowRead {
            concurrent: concurrent.clone(),
            peak: peak.clone(),
        })]);
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![
                ("t1", "read", serde_json::json!({})),
                ("t2", "read", serde_json::json!({})),
                ("t3", "read", serde_json::json!({})),
            ]),
            FakeLlmClient::say("done"),
        ]);
        let started = std::time::Instant::now();
        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        TurnEngine::new(BudgetConfig::default())
            .run_turn(&llm, &registry, &ctx, &mut history, "go", "sys", "m", CancellationToken::new(), &emit)
            .await;
        let elapsed = started.elapsed();
        assert!(peak.load(std::sync::atomic::Ordering::SeqCst) >= 2, "reads must overlap");
        assert!(
            elapsed < std::time::Duration::from_millis(300),
            "3x120ms serial would be 360ms; concurrent must be faster: {elapsed:?}"
        );
        // 事件序：starts 全部先于 ends（段内响应序）
        let evs = collect(&mut rx);
        let kinds: Vec<&str> = evs.iter().filter_map(|e| match e {
            AgentEvent::ToolStart { .. } => Some("start"),
            AgentEvent::ToolEnd { .. } => Some("end"),
            _ => None,
        }).collect();
        assert_eq!(kinds, vec!["start", "start", "start", "end", "end", "end"]);
        // tool_result 按响应序回填
        let ids: Vec<&String> = history.iter().filter_map(|m| match &m.content[0] {
            ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id),
            _ => None,
        }).collect();
        assert_eq!(ids, vec!["t1", "t2", "t3"]);
    }

    #[tokio::test]
    async fn write_serializes_against_neighboring_readonly_run() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tx, mut rx) = setup(dir.path());
        let registry = ToolRegistry::from_tools(vec![
            Arc::new(SlowRead {
                concurrent: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                peak: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }),
            Arc::new(crate::tools::fs_ops::WriteTool),
        ]);
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![
                ("t1", "read", serde_json::json!({})),
                ("t2", "write", serde_json::json!({"path": "w.txt", "content": "x"})),
                ("t3", "read", serde_json::json!({})),
            ]),
            FakeLlmClient::say("done"),
        ]);
        let mut history = Vec::new();
        let emit = TurnEmit::new("s1", &tx);
        TurnEngine::new(BudgetConfig::default())
            .run_turn(&llm, &registry, &ctx, &mut history, "go", "sys", "m", CancellationToken::new(), &emit)
            .await;
        // 事件序 = 响应序：start,end（读1）→ start,end（写）→ start,end（读2）
        let evs = collect(&mut rx);
        let kinds: Vec<&str> = evs.iter().filter_map(|e| match e {
            AgentEvent::ToolStart { tool_name, .. } => Some(match tool_name.as_str() { "read" => "r", _ => "w" }),
            AgentEvent::ToolEnd { tool_name, .. } => Some(match tool_name.as_str() { "read" => "R", _ => "W" }),
            _ => None,
        }).collect();
        assert_eq!(kinds, vec!["r", "R", "w", "W", "r", "R"]);
        assert!(dir.path().join("w.txt").exists(), "write must have run");
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
