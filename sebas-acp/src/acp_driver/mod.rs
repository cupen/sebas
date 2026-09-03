//! The generic ACP driver: spawns a native Agent-Client-Protocol agent
//! (`gemini --acp`, `codex-acp`, …) and drives it through
//! `agent-client-protocol` v1, emitting the crate-level `AcpEvent` vocabulary.
//!
//! `AcpAgent` owns the child process (dropping the connection terminates its
//! process group), so cancel = dropping the run loop, which the manager's
//! wrapper awaits.

mod codec;

use crate::agent_driver::{AgentDriver, DriverConfig, DriverError, DriverHandle};
use crate::session::{AcpCommand, AcpEvent, Decision};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, InitializeRequest, NewSessionRequest, PermissionOption,
    PermissionOptionKind, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionNotification, TextContent,
};
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Agent, Client, ConnectionTo};
use std::collections::HashMap;
use tokio::sync::oneshot;

pub struct AcpDriver;

#[async_trait::async_trait]
impl AgentDriver for AcpDriver {
    async fn spawn(&self, cfg: DriverConfig) -> Result<DriverHandle, DriverError> {
        let DriverConfig {
            kind_slug,
            command,
            work_dir,
            extra_env,
            session_id,
            resume,
            startup_timeout,
            evt_tx,
            mut cmd_rx,
            mut cancel_rx,
            pending_perms,
            terminal_sent,
        } = cfg;

        if resume {
            // ACP session persistence (session/load across a daemon restart)
            // is not wired yet; an honest fresh start beats pretending we
            // resumed. The manager's fallback semantics still apply.
            tracing::warn!(
                kind = %kind_slug,
                "ACP resume not implemented; starting a fresh session"
            );
        }

        // Build the transport (spawns the subprocess; drop = process-group
        // termination per the SDK's AcpAgent contract).
        let mut argv = command.into_iter();
        let exe = argv
            .next()
            .ok_or_else(|| DriverError::NotFound("empty command".to_string()))?;
        let args: Vec<String> = argv.collect();
        let mut agent_cfg = AcpAgentConfig::new(exe).args(args);
        for (k, v) in extra_env {
            agent_cfg = agent_cfg.env(k, v);
        }
        let agent = AcpAgent::new(agent_cfg);

        let routing_id = session_id.clone();
        let cwd = work_dir.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "/".to_string())
        });

        let run = async move {
            let result = Client.builder()
                .on_receive_notification(
                    {
                        let evt_tx = evt_tx.clone();
                        let routing_id = routing_id.clone();
                        let mut tool_names: HashMap<String, String> = HashMap::new();
                        async move |notification: SessionNotification, _cx| {
                            for evt in codec::translate_notification(
                                &routing_id,
                                &mut tool_names,
                                &notification,
                            ) {
                                if evt_tx.send(evt).await.is_err() {
                                    break;
                                }
                            }
                            Ok(())
                        }
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .on_receive_request(
                    {
                        let evt_tx = evt_tx.clone();
                        let routing_id = routing_id.clone();
                        let kind_slug = kind_slug.clone();
                        let pending_perms = pending_perms.clone();
                        async move |request: RequestPermissionRequest, responder, _cx| {
                            let raw_id = request.tool_call.tool_call_id.to_string();
                            let request_id = format!("{kind_slug}:{raw_id}");
                            let tool_name = request
                                .tool_call
                                .fields
                                .title
                                .clone()
                                .unwrap_or_else(|| "tool".to_string());
                            let args = request
                                .tool_call
                                .fields
                                .raw_input
                                .clone()
                                .unwrap_or(serde_json::Value::Null);
                            let (tx, rx) = oneshot::channel();
                            pending_perms.lock().await.insert(request_id.clone(), tx);
                            let _ = evt_tx
                                .send(AcpEvent::PermissionRequest {
                                    session_id: routing_id.clone(),
                                    request_id: request_id.clone(),
                                    tool_name,
                                    args,
                                })
                                .await;
                            // Park until the manager resolves the oneshot; on
                            // drop (no answerer) fail closed.
                            let decision = match rx.await {
                                Ok(d) => d,
                                Err(_) => Decision::Deny,
                            };
                            let response = map_decision(&decision, &request.options);
                            responder.respond(response)?;
                            Ok(())
                        }
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_with(agent, |cx: ConnectionTo<Agent>| async move {
                    cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                        .block_task()
                        .await?;
                    let acp_session_id = cx
                        .send_request(NewSessionRequest::new(cwd.clone()))
                        .block_task()
                        .await?
                        .session_id;

                    loop {
                        tokio::select! {
                            _ = &mut cancel_rx => break,
                            cmd = cmd_rx.recv() => match cmd {
                                Some(AcpCommand::CreateSession { prompt, .. })
                                | Some(AcpCommand::ContinueSession { prompt, .. }) => {
                                    cx.send_request(PromptRequest::new(
                                        acp_session_id.clone(),
                                        vec![ContentBlock::Text(TextContent::new(prompt))],
                                    ))
                                    .block_task()
                                    .await?;
                                }
                                Some(AcpCommand::Cancel { .. }) => {
                                    let _ = cx
                                        .send_notification(CancelNotification::new(acp_session_id.clone()));
                                }
                                Some(AcpCommand::PermissionReply { .. }) => {
                                    // Replies ride the pending map, not the
                                    // command channel.
                                }
                                None => break,
                            }
                        }
                    }
                    Ok(())
                })
                .await;

            if let Err(e) = result
                && !terminal_sent.load(std::sync::atomic::Ordering::SeqCst)
            {
                terminal_sent.store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = evt_tx
                    .send(AcpEvent::Error {
                        session_id: routing_id.clone(),
                        message: format!("acp driver error: {e:#}"),
                        terminal: true,
                    })
                    .await;
            }
        };

        let _ = startup_timeout; // the SDK's connect() is driven inside `run`;
                                 // timeout enforcement is a follow-up (R3).

        Ok(DriverHandle {
            session_id,
            resumed: false,
            run: Box::pin(run),
        })
    }
}

/// Map a sebas [`Decision`] onto an ACP permission response by selecting the
/// offered option whose kind matches; fall back to `Cancelled` (deny) when no
/// option matches or none are offered.
fn map_decision(
    decision: &Decision,
    options: &[PermissionOption],
) -> RequestPermissionResponse {
    let wanted = match decision {
        Decision::AllowOnce => PermissionOptionKind::AllowOnce,
        Decision::AllowSession => PermissionOptionKind::AllowAlways,
        Decision::Deny => PermissionOptionKind::RejectOnce,
    };
    if let Some(opt) = options.iter().find(|o| o.kind == wanted) {
        return RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new(opt.option_id.clone()),
        ));
    }
    // No exact match: deny is honest (Cancelled), allow falls back to the
    // first offered option rather than silently failing.
    match decision {
        Decision::Deny => {
            RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled)
        }
        _ => options
            .first()
            .map(|opt| {
                RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
                    SelectedPermissionOutcome::new(opt.option_id.clone()),
                ))
            })
            .unwrap_or_else(|| {
                RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled)
            }),
    }
}
