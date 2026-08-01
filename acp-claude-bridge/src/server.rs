//! ACP server side: registers handlers on `agent-client-protocol`'s builder
//! and translates incoming requests to/from the ClaudeDriver and permission
//! broker.

use crate::claude::driver::ClaudeDriver;
use crate::notifications;
use crate::permission::PermissionDecision;
use crate::translator;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, InitializeRequest, InitializeResponse,
    LoadSessionRequest, NewSessionRequest, NewSessionResponse, PermissionOption,
    PermissionOptionKind, PromptCapabilities, PromptRequest, PromptResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SessionId, StopReason, ToolCallUpdate, ToolCallUpdateFields,
};
use agent_client_protocol::{on_receive_notification, on_receive_request, Agent, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Map a `PermissionOptionId` (as string) to a `PermissionDecision`.
/// `allow_*` → Allow, anything else → Deny. 字符串约定必须与
/// `acp-claude/manager.rs:320-329` 保持一致；该侧改字符串时要同步通知 bridge。
fn option_id_to_decision(id: &str) -> crate::permission::PermissionDecision {
    if id.starts_with("allow_") {
        crate::permission::PermissionDecision::Allow
    } else {
        crate::permission::PermissionDecision::Deny
    }
}

/// 固定 3 个 permission 选项，与 acp-claude 端一致：
/// `allow_once` / `allow_always` / `reject_once`。
fn build_permission_options() -> Vec<PermissionOption> {
    vec![
        PermissionOption::new("allow_once", "Allow once", PermissionOptionKind::AllowOnce),
        PermissionOption::new("allow_always", "Allow for this chat", PermissionOptionKind::AllowAlways),
        PermissionOption::new("reject_once", "Deny", PermissionOptionKind::RejectOnce),
    ]
}

pub async fn run(
    mut claude: ClaudeDriver,
    perm_tx: mpsc::Sender<PermissionDecision>,
) -> anyhow::Result<()> {
    // 串行化所有 prompt handler：claude 子进程是单 stream，一次只允许一个 pump 在读
    let gate: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
    // CancelNotification 只置位；当前不向 claude 子进程发中断信号（范围外）
    let cancel_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    Agent
        .builder()
        .name("claude-acp-bridge")
        .on_receive_request(
            async move |req: InitializeRequest, responder, _cx| {
                let caps = AgentCapabilities::new()
                    .load_session(false)
                    .prompt_capabilities(
                        PromptCapabilities::new()
                            .image(false)
                            .audio(false)
                            .embedded_context(false),
                    );
                responder.respond(
                    InitializeResponse::new(req.protocol_version).agent_capabilities(caps),
                )
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |req: NewSessionRequest, responder, _cx| {
                let id = SessionId::new(uuid::Uuid::new_v4().to_string());
                responder.respond(NewSessionResponse::new(id))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |req: LoadSessionRequest, responder, _cx| {
                // Bridge intentionally returns "session not found" — sebas
                // already handles this by falling back to SpawnAcp with a
                // fresh session.
                responder.respond_with_error(agent_client_protocol::Error::new(
                    -32000,
                    "loadSession not supported by bridge",
                ))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            {
                let gate = gate.clone();
                let cancel_flag = cancel_flag.clone();
                async move |req: PromptRequest, responder, cx| {
                    let session_id = req.session_id.0.to_string();
                    let text = req
                        .prompt
                        .iter()
                        .filter_map(|b| match b {
                            agent_client_protocol::schema::v1::ContentBlock::Text(t) => {
                                Some(t.text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    tracing::info!(session_id=%session_id, text_len=text.len(), "prompt received");

                    cancel_flag.store(false, Ordering::SeqCst);
                    let _guard = gate.lock().await;

                    if let Err(e) = claude.send_user(&text).await {
                        tracing::warn!(error=%e, "send_user failed");
                        let _ = responder.respond_with_error(
                            agent_client_protocol::util::internal_error(
                                "claude send_user failed",
                            ),
                        );
                        return Ok(());
                    }

                    let mut stop_reason = StopReason::EndTurn;
                    loop {
                        let Some(event) = claude.next_event().await else {
                            tracing::warn!(session_id=%session_id, "driver EOF before TurnEnd");
                            break;
                        };
                        if cancel_flag.load(Ordering::SeqCst) {
                            stop_reason = StopReason::Cancelled;
                            break;
                        }
                        // ToolUse 拦截：先发 ToolCall 通知，再同步等 sebas RequestPermissionResponse。
                        // spec §4：失败一律 Deny（send_request_to Err / Response 非 Selected / option_id 不以 "allow_" 开头）。
                        // 拦截后该 event 不再走 translator（ToolCall 通知已用 from_update 显式发出）。
                        let updates: Vec<crate::translator::TranslatedUpdate> = match &event {
                            crate::claude::StreamEvent::ToolUse { id, name, input } => {
                                let tool_call_update = ToolCallUpdate::new(
                                    id.clone(),
                                    ToolCallUpdateFields::new()
                                        .title(name.clone())
                                        .raw_input(input.clone()),
                                );
                                // 1) emit ToolCall 通知
                                let notif = notifications::from_update(
                                    &session_id,
                                    crate::translator::TranslatedUpdate::ToolCall {
                                        id: id.clone(),
                                        title: name.clone(),
                                        raw_input: input.clone(),
                                    },
                                );
                                if let Some(n) = notif {
                                    if let Err(e) = cx.send_notification(n) {
                                        tracing::warn!(error=%e, tool_id=%id, "tool_call notification failed");
                                    }
                                }
                                // 2) 同步等 sebas 决策
                                let req = RequestPermissionRequest::new(
                                    SessionId::new(session_id.clone()),
                                    tool_call_update,
                                    build_permission_options(),
                                );
                                let decision = match cx
                                    .send_request_to(agent_client_protocol::Client, req)
                                    .block_task()
                                    .await
                                {
                                    Ok(resp) => match resp.outcome {
                                        RequestPermissionOutcome::Selected(sel) => {
                                            tracing::info!(tool_id=%id, option=%sel.option_id.0, "permission selected");
                                            option_id_to_decision(sel.option_id.0.as_ref())
                                        }
                                        other => {
                                            tracing::warn!(tool_id=%id, ?other, "permission outcome not Selected → Deny");
                                            crate::permission::PermissionDecision::Deny
                                        }
                                    },
                                    Err(e) => {
                                        tracing::warn!(error=%e, tool_id=%id, "send_request_to failed → Deny");
                                        crate::permission::PermissionDecision::Deny
                                    }
                                };
                                // 3) 写 broker → hook unblock → claude 继续
                                if let Err(e) = perm_tx.send(decision.clone()).await {
                                    tracing::warn!(error=%e, "perm_tx.send failed");
                                }
                                // 4) 拒绝时补 ToolCallUpdate Failed
                                if decision == crate::permission::PermissionDecision::Deny {
                                    let denied = notifications::from_update(
                                        &session_id,
                                        crate::translator::TranslatedUpdate::ToolCallUpdate {
                                            id: id.clone(),
                                            status: crate::translator::ToolStatus::Failed,
                                            raw_output: Some("denied by sebas".into()),
                                        },
                                    );
                                    if let Some(n) = denied {
                                        if let Err(e) = cx.send_notification(n) {
                                            tracing::warn!(error=%e, tool_id=%id, "denied notification failed");
                                        }
                                    }
                                }
                                Vec::new() // 拦截后不再走 translator
                            }
                            _ => translator::translate(event.clone()),
                        };
                        for update in updates {
                            if let Some(notif) =
                                notifications::from_update(&session_id, update)
                            {
                                if let Err(e) = cx.send_notification(notif) {
                                    tracing::warn!(error=%e, "send_notification failed");
                                }
                            }
                        }
                        if let crate::claude::StreamEvent::TurnEnd { stop_reason: sr } = event {
                            stop_reason = notifications::acp_stop_reason(sr);
                            break;
                        }
                    }
                    let _ = responder.respond(PromptResponse::new(stop_reason));
                    Ok(())
                }
            },
            on_receive_request!(),
        )
        .on_receive_notification(
            {
                let cancel_flag = cancel_flag.clone();
                async move |_notif: CancelNotification, _cx| {
                    cancel_flag.store(true, Ordering::SeqCst);
                    tracing::info!("cancel received");
                    Ok(())
                }
            },
            on_receive_notification!(),
        )
        .connect_to(Stdio::new())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::PermissionDecision;

    #[test]
    fn option_id_allow_once_maps_to_allow() {
        assert_eq!(option_id_to_decision("allow_once"), PermissionDecision::Allow);
    }

    #[test]
    fn option_id_allow_always_maps_to_allow() {
        assert_eq!(option_id_to_decision("allow_always"), PermissionDecision::Allow);
    }

    #[test]
    fn option_id_reject_once_maps_to_deny() {
        assert_eq!(option_id_to_decision("reject_once"), PermissionDecision::Deny);
    }

    #[test]
    fn option_id_unknown_maps_to_deny() {
        assert_eq!(option_id_to_decision("mystery"), PermissionDecision::Deny);
    }

    #[test]
    fn build_permission_options_returns_three_stable_ids() {
        let opts = build_permission_options();
        assert_eq!(opts.len(), 3);
        let ids: Vec<&str> = opts.iter().map(|o| o.option_id.0.as_ref()).collect();
        assert_eq!(ids, vec!["allow_once", "allow_always", "reject_once"]);
    }
}