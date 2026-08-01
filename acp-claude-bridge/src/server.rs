//! ACP server side: registers handlers on `agent-client-protocol`'s builder
//! and translates incoming requests to/from the ClaudeDriver and permission
//! broker.

use crate::claude::driver::ClaudeDriver;
use crate::permission::PermissionDecision;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, InitializeRequest, InitializeResponse,
    LoadSessionRequest, NewSessionRequest, NewSessionResponse, PromptRequest,
    PromptResponse, SessionId, StopReason,
};
use agent_client_protocol::{on_receive_notification, on_receive_request, Agent, Stdio};
use tokio::sync::mpsc;

pub async fn run(
    mut claude: ClaudeDriver,
    perm_tx: mpsc::Sender<PermissionDecision>,
) -> anyhow::Result<()> {
    let _ = perm_tx;
    Agent
        .builder()
        .name("claude-acp-bridge")
        .on_receive_request(
            async move |req: InitializeRequest, responder, _cx| {
                let caps = AgentCapabilities::new()
                    .load_session(false)
                    .prompt_capabilities(
                        agent_client_protocol::schema::v1::PromptCapabilities::new()
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
            async move |req: PromptRequest, responder, _cx| {
                let session_id = req.session_id.clone();
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
                responder.respond(PromptResponse::new(StopReason::EndTurn))
            },
            on_receive_request!(),
        )
        .on_receive_notification(
            async move |_notif: CancelNotification, _cx| {
                tracing::info!("cancel received");
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(Stdio::new())
        .await?;
    Ok(())
}
