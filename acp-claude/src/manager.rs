//! `SessionManager` — public API for spawning per-session ACP agents
//! backed by the official `agent-client-protocol` v2 SDK.

use crate::session::{
    translate_update, AcpCommand, AcpEvent, AcpSessionHandle, Decision, ResponderSlot, SessionMeta,
};
use agent_client_protocol::schema::v1::{
    CancelNotification, ClientCapabilities, Implementation, InitializeRequest, PermissionOptionId,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionId as SdkSessionId, SessionNotification, StopReason,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{
    on_receive_request, AcpAgent, AcpAgentConfig, Agent, Client, ConnectionTo, Dispatch,
    SessionMessage,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

pub struct SessionManager {
    inner: Arc<Mutex<HashMap<String, SessionMeta>>>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Spawn a per-session ACP agent subprocess, complete the
    /// `initialize` and `session/new` handshake against the official SDK,
    /// and return the SDK-issued `session_id` that callers use as the
    /// routing key for `send`/`event_rx`/`kill`.
    ///
    /// `path` and `args` mirror `tokio::process::Command::new(...).args(...)`.
    /// `work_dir` is accepted for source-compat with the prior hand-rolled
    /// spawn but the v2 SDK does not yet expose a per-process cwd, so
    /// the child inherits the parent's cwd. The session's working
    /// directory is still reported to the agent via `NewSessionRequest`
    /// (defaulting to "." if None) so prompts and tool calls see a
    /// sensible path.
    ///
    /// `prompt` is part of the public API surface (the prior hand-rolled
    /// `SessionManager::create_session` accepted it) but is **not** sent
    /// here — the agent connection is established but no message is
    /// pushed until the caller dispatches `AcpCommand::CreateSession`
    /// (or `ContinueSession`) through `send`. Keeping the param unused
    /// is intentional: the prompt is forwarded exactly once via the
    /// command channel.
    pub async fn create_session(
        &self,
        path: &str,
        args: Vec<String>,
        work_dir: Option<String>,
        _prompt: String,
    ) -> anyhow::Result<String> {
        let agent = AcpAgent::new(AcpAgentConfig::new(path).args(args));

        let (cmd_tx, cmd_rx) = mpsc::channel::<AcpCommand>(64);
        let (evt_tx, evt_rx) = mpsc::channel::<AcpEvent>(256);
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (init_tx, init_rx) = oneshot::channel::<String>();
        let pending_responders: Arc<Mutex<HashMap<String, ResponderSlot>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let pending_responders_for_cb = pending_responders.clone();
        let evt_tx_for_cb = evt_tx.clone();
        let cwd = work_dir.clone().unwrap_or_else(|| ".".to_string());

        tokio::spawn(async move {
            let result = run_session(
                agent,
                cwd,
                cmd_rx,
                evt_tx,
                cancel_rx,
                init_tx,
                pending_responders_for_cb,
                evt_tx_for_cb,
            )
            .await;
            if let Err(e) = result {
                tracing::error!(?e, "acp session task ended with error");
            }
        });

        let session_id = init_rx
            .await
            .map_err(|_| anyhow::anyhow!("acp session closed before session/new"))?;

        let handle = AcpSessionHandle {
            session_id: session_id.clone(),
            cmd_tx,
            evt_rx: Arc::new(Mutex::new(evt_rx)),
            cancel_tx: Some(cancel_tx),
            pending_responders,
        };
        self.inner.lock().await.insert(
            session_id.clone(),
            SessionMeta {
                session_id: session_id.clone(),
                handle,
            },
        );

        Ok(session_id)
    }

    /// Stop one session. Drops the cancel sender; the background task
    /// selects on `cancel_rx` and exits, dropping the SDK connection
    /// (and the child process).
    pub async fn kill(&self, session_id: &str) {
        if let Some(meta) = self.inner.lock().await.remove(session_id) {
            if let Some(tx) = meta.handle.cancel_tx {
                let _ = tx.send(());
            }
        }
    }

    /// Dispatch an `AcpCommand` to its handler.
    ///
    /// - `PermissionReply` looks up the previously-captured responder
    ///   for the request_id and invokes it with the user's decision.
    ///   Does not touch the SDK.
    /// - `Cancel` sends a `session/cancel` notification through the
    ///   command channel; the background task forwards it to the agent
    ///   and exits.
    /// - Everything else (CreateSession, ContinueSession) is forwarded
    ///   to the command channel for the background task to handle.
    pub async fn send(&self, session_id: &str, cmd: AcpCommand) -> anyhow::Result<()> {
        let g = self.inner.lock().await;
        let m = g
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("unknown session"))?;

        match &cmd {
            AcpCommand::PermissionReply {
                request_id,
                decision,
                ..
            } => {
                let mut map = m.handle.pending_responders.lock().await;
                let slot = map.remove(request_id);
                drop(map);
                let Some(slot) = slot else {
                    tracing::warn!(
                        request_id,
                        "no pending responder; dropping permission reply"
                    );
                    return Ok(());
                };
                let response =
                    RequestPermissionResponse::new(decision_to_outcome(decision.clone()));
                slot(response)
                    .map_err(|e| anyhow::anyhow!("failed to send permission response: {e}"))?;
                Ok(())
            }
            other => m
                .handle
                .cmd_tx
                .send(other.clone())
                .await
                .map_err(|e| anyhow::anyhow!("send to session cmd channel: {e}")),
        }
    }

    pub async fn next_event(&self, session_id: &str) -> Option<AcpEvent> {
        let g = self.inner.lock().await;
        let m = g.get(session_id)?;
        let mut rx = m.handle.evt_rx.lock().await;
        rx.recv().await
    }

    pub async fn event_rx(&self, session_id: &str) -> Option<Arc<Mutex<mpsc::Receiver<AcpEvent>>>> {
        self.inner
            .lock()
            .await
            .get(session_id)
            .map(|m| m.handle.evt_rx.clone())
    }

    /// Stop every live session. Called on daemon shutdown so child
    /// processes are signalled before the state snapshot is written.
    pub async fn kill_all(&self) {
        let metas: Vec<SessionMeta> = self.inner.lock().await.drain().map(|(_, m)| m).collect();
        for m in metas {
            if let Some(tx) = m.handle.cancel_tx {
                let _ = tx.send(());
            }
        }
    }
}

fn decision_to_outcome(decision: Decision) -> RequestPermissionOutcome {
    let option_id = match decision {
        Decision::AllowOnce => "allow_once",
        Decision::AllowSession => "allow_always",
        Decision::Deny => "reject_once",
    };
    RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(PermissionOptionId::new(
        option_id.to_string(),
    )))
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    agent: AcpAgent,
    cwd: String,
    cmd_rx: mpsc::Receiver<AcpCommand>,
    evt_tx: mpsc::Sender<AcpEvent>,
    cancel_rx: oneshot::Receiver<()>,
    init_tx: oneshot::Sender<String>,
    pending_responders: Arc<Mutex<HashMap<String, ResponderSlot>>>,
    evt_tx_for_cb: mpsc::Sender<AcpEvent>,
) -> Result<(), agent_client_protocol::Error> {
    Client
        .builder()
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _cx| {
                let request_id = request.tool_call.tool_call_id.0.to_string();
                let tool_name = request.tool_call.fields.title.clone().unwrap_or_default();
                let args = request
                    .tool_call
                    .fields
                    .raw_input
                    .clone()
                    .unwrap_or(serde_json::Value::Null);
                let session_id_in_request = request.session_id.0.to_string();
                let slot: ResponderSlot = Box::new(move |response: RequestPermissionResponse| {
                    responder.respond(response)
                });
                pending_responders
                    .lock()
                    .await
                    .insert(request_id.clone(), slot);
                let _ = evt_tx_for_cb
                    .send(AcpEvent::PermissionRequest {
                        session_id: session_id_in_request,
                        request_id,
                        tool_name,
                        args,
                    })
                    .await;
                Ok(())
            },
            on_receive_request!(),
        )
        .connect_with(agent, move |cx: ConnectionTo<Agent>| async move {
            run_main(cx, cwd, init_tx, evt_tx, cmd_rx, cancel_rx).await
        })
        .await
}

#[allow(clippy::too_many_arguments)]
async fn run_main(
    cx: ConnectionTo<Agent>,
    cwd: String,
    init_tx: oneshot::Sender<String>,
    evt_tx: mpsc::Sender<AcpEvent>,
    mut cmd_rx: mpsc::Receiver<AcpCommand>,
    cancel_rx: oneshot::Receiver<()>,
) -> Result<(), agent_client_protocol::Error> {
    // 1) initialize — protocol version 1.
    cx.send_request(
        InitializeRequest::new(ProtocolVersion::V1)
            .client_capabilities(ClientCapabilities::default())
            .client_info(Implementation::new("sebas", "0.1.0")),
    )
    .block_task()
    .await?;

    // 2) session/new — exactly once, via the SDK session builder. The
    //    SDK-issued session id is the single routing id for the whole
    //    pipeline (prompts, updates, permission requests, cancel).
    let mut session = cx
        .build_session(std::path::PathBuf::from(&cwd))
        .block_task()
        .start_session()
        .await?;
    let session_id = session.session_id().0.to_string();
    let _ = init_tx.send(session_id.clone());

    // 4) Read loop. The session is long-lived: each `StopReason::EndTurn`
    //    is forwarded as a `Finished` event but the connection is kept
    //    alive so the caller can issue `ContinueSession` commands for
    //    follow-up prompts. The loop exits (and the SDK connection
    //    drops, SIGKILL-ing the child) only when:
    //      - `cancel_rx` fires (`kill_all` or `kill`)
    //      - a transport error occurs
    //      - the cmd channel closes (consumer dropped the manager)
    //      - a `Refusal` stop reason is reported
    //    `AcpCommand::Cancel` does NOT exit the loop — it cancels the
    //    current turn and the session stays alive.
    let mut cancel_rx = cancel_rx;
    let mut should_exit = false;

    while !should_exit {
        tokio::select! {
            biased;
            _ = &mut cancel_rx => {
                // Kill path — `kill` or `kill_all` was called. This
                // terminates the entire session (connection drop →
                // child SIGKILL).
                let _ = cx.send_notification(CancelNotification::new(SdkSessionId::new(session_id.clone())));
                let _ = evt_tx
                    .send(AcpEvent::Finished { session_id: session_id.clone() })
                    .await;
                break;
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(AcpCommand::Cancel { .. }) => {
                        // Cancel the current TURN only; the session stays
                        // alive. The agent answers with StopReason::Cancelled
                        // which the read loop maps to a turn-level Finished.
                        let _ = cx.send_notification(CancelNotification::new(
                            SdkSessionId::new(session_id.clone()),
                        ));
                    }
                    Some(AcpCommand::CreateSession { prompt, .. }) => {
                        // `CreateSession` is the single channel through
                        // which the initial prompt reaches the agent;
                        // `create_session` deliberately does not push
                        // one on its own, so this is the only send.
                        if session.send_prompt(prompt).is_err() {
                            let _ = evt_tx
                                .send(AcpEvent::Error {
                                    session_id: session_id.clone(),
                                    message: "session/prompt failed".into(),
                                })
                                .await;
                            break;
                        }
                    }
                    Some(AcpCommand::ContinueSession { prompt, .. }) => {
                        if session.send_prompt(prompt).is_err() {
                            let _ = evt_tx
                                .send(AcpEvent::Error {
                                    session_id: session_id.clone(),
                                    message: "session/prompt failed".into(),
                                })
                                .await;
                            break;
                        }
                    }
                    Some(AcpCommand::PermissionReply { .. }) => {
                        // PermissionReply goes through a different path
                        // in SessionManager::send (it talks directly to
                        // the captured ResponderSlot). It shouldn't
                        // arrive here.
                        tracing::debug!("ignoring unexpected command on session channel");
                    }
                    None => break,
                }
            }
            update = session.read_update() => {
                match update {
                    Ok(SessionMessage::SessionMessage(message)) => {
                        if let Some(event) = translate_dispatch(&session_id, message) {
                            if evt_tx.send(event).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(SessionMessage::StopReason(reason)) => {
                        if matches!(reason, StopReason::Refusal) {
                            let _ = evt_tx
                                .send(AcpEvent::Error {
                                    session_id: session_id.clone(),
                                    message: "agent refused".into(),
                                })
                                .await;
                            should_exit = true;
                        } else {
                            let _ = evt_tx
                                .send(translate_stop_reason(&session_id, reason))
                                .await;
                        }
                    }
                    Ok(_) => {
                        // New SessionMessage variants added by the SDK
                        // in future versions; ignore for now.
                    }
                    Err(e) => {
                        let _ = evt_tx
                            .send(AcpEvent::Error {
                                session_id: session_id.clone(),
                                message: format!("acp transport error: {e}"),
                            })
                            .await;
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Extract a `SessionNotification` from a generic `Dispatch` and
/// translate it. The agent's only session-scope notification is
/// `SessionNotification` (`session/update`); we deserialize the
/// `params` payload of the untyped notification directly.
fn translate_dispatch(session_id: &str, message: Dispatch) -> Option<AcpEvent> {
    let untyped = match message {
        Dispatch::Notification(n) => n,
        Dispatch::Request(_, _) | Dispatch::Response(_, _) => return None,
    };
    if untyped.method() != "session/update" {
        return None;
    }
    let notif: SessionNotification = serde_json::from_value(untyped.params().clone()).ok()?;
    translate_update(session_id, &notif)
}

fn translate_stop_reason(session_id: &str, reason: StopReason) -> AcpEvent {
    match reason {
        StopReason::EndTurn
        | StopReason::MaxTokens
        | StopReason::MaxTurnRequests
        | StopReason::Cancelled => AcpEvent::Finished {
            session_id: session_id.to_string(),
        },
        StopReason::Refusal => AcpEvent::Error {
            session_id: session_id.to_string(),
            message: "agent refused".into(),
        },
        _ => AcpEvent::Finished {
            session_id: session_id.to_string(),
        },
    }
}
