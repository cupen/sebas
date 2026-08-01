//! ACP server side: registers handlers on `agent-client-protocol`'s builder
//! and translates incoming requests to/from the ClaudeDriver and permission
//! broker.

use crate::claude::driver::ClaudeDriver;
use crate::notifications;
use crate::permission::PermissionDecision;
use crate::translator;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, InitializeRequest, InitializeResponse,
    LoadSessionRequest, NewSessionRequest, NewSessionResponse, PromptCapabilities,
    PromptRequest, PromptResponse, SessionId, StopReason,
};
use agent_client_protocol::{on_receive_notification, on_receive_request, Agent, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

pub async fn run(
    mut claude: ClaudeDriver,
    perm_tx: mpsc::Sender<PermissionDecision>,
) -> anyhow::Result<()> {
    let _ = perm_tx; // 下一 ticket 接通 permission broker；本 ticket 保持参数签名
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
                        for update in translator::translate(event.clone()) {
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