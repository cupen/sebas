use crate::error::{Result, SebasError};
use crate::watchdog::control::{
    Actor, ControlEvent, ControlRequest, ControlResponse, ControlService, UpdateKind,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlEnvelope {
    pub version: u16,
    #[serde(default)]
    pub request_id: String,
    pub actor: RpcActor,
    pub request: RpcControlRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcActor {
    Cli { uid: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum RpcControlRequest {
    Status,
    EventsSince { seq: u64 },
    Update { dev: bool, dry_run: bool },
    Rollback { dry_run: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcControlResponse {
    Accepted {
        operation_id: String,
        status: String,
    },
    Rejected {
        code: String,
        message: String,
    },
    Events {
        events: Vec<RpcControlEvent>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcControlEvent {
    pub seq: u64,
    pub operation_id: String,
    pub kind: String,
    pub public_message: String,
}

pub fn default_socket_path() -> PathBuf {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime_dir).join("sebas/control.sock");
    }
    std::env::temp_dir().join("sebas-control.sock")
}

pub async fn serve(path: PathBuf, control: Arc<Mutex<ControlService>>) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if path.exists() {
        let _ = tokio::fs::remove_file(&path).await;
    }
    let listener = UnixListener::bind(&path)?;
    set_socket_permissions(&path)?;

    loop {
        let (stream, _) = listener.accept().await?;
        let control = control.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_stream(stream, control).await {
                warn!("control RPC connection failed: {e}");
            }
        });
    }
}

pub async fn request(path: &Path, envelope: &ControlEnvelope) -> Result<RpcControlResponse> {
    let stream = UnixStream::connect(path).await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let json = serde_json::to_string(envelope)
        .map_err(|e| SebasError::Upgrade(format!("control RPC serialize failed: {e}")))?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;

    let mut line = String::new();
    reader.read_line(&mut line).await?;
    if line.is_empty() {
        return Err(SebasError::Upgrade(
            "control RPC closed without response".into(),
        ));
    }
    serde_json::from_str(&line)
        .map_err(|e| SebasError::Upgrade(format!("control RPC parse response failed: {e}")))
}

async fn handle_stream(stream: UnixStream, control: Arc<Mutex<ControlService>>) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    if line.is_empty() {
        return Ok(());
    }

    let response = match serde_json::from_str::<ControlEnvelope>(&line) {
        Ok(envelope) => handle_envelope(envelope, control).await,
        Err(e) => RpcControlResponse::Rejected {
            code: "invalid_request".into(),
            message: format!("invalid control RPC request: {e}"),
        },
    };
    let json = serde_json::to_string(&response)
        .map_err(|e| SebasError::Upgrade(format!("control RPC serialize response failed: {e}")))?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

async fn handle_envelope(
    envelope: ControlEnvelope,
    control: Arc<Mutex<ControlService>>,
) -> RpcControlResponse {
    if envelope.version != 1 {
        return RpcControlResponse::Rejected {
            code: "unsupported_version".into(),
            message: format!("unsupported control RPC version {}", envelope.version),
        };
    }

    match envelope.request {
        RpcControlRequest::Status => {
            accept_control_request(control, envelope.actor, ControlRequest::Status).await
        }
        RpcControlRequest::EventsSince { seq } => {
            let control = control.lock().await;
            RpcControlResponse::Events {
                events: control
                    .events_since(seq)
                    .into_iter()
                    .map(RpcControlEvent::from)
                    .collect(),
            }
        }
        RpcControlRequest::Update { dev, dry_run } => {
            accept_control_request(
                control,
                envelope.actor,
                ControlRequest::Update {
                    kind: if dev {
                        UpdateKind::Dev
                    } else {
                        UpdateKind::Release
                    },
                    dry_run,
                    target: None,
                },
            )
            .await
        }
        RpcControlRequest::Rollback { dry_run } => {
            accept_control_request(
                control,
                envelope.actor,
                ControlRequest::Rollback { dry_run },
            )
            .await
        }
    }
}

async fn accept_control_request(
    control: Arc<Mutex<ControlService>>,
    actor: RpcActor,
    request: ControlRequest,
) -> RpcControlResponse {
    let mut control = control.lock().await;
    let actor = match actor {
        RpcActor::Cli { uid } => Actor::Cli { uid },
    };
    match control.accept(actor, request) {
        ControlResponse::Accepted {
            operation_id,
            status,
        } => RpcControlResponse::Accepted {
            operation_id,
            status: format!("{status:?}"),
        },
        ControlResponse::Rejected { code, message } => RpcControlResponse::Rejected {
            code: format!("{code:?}"),
            message,
        },
    }
}

impl From<ControlEvent> for RpcControlEvent {
    fn from(event: ControlEvent) -> Self {
        Self {
            seq: event.seq,
            operation_id: event.operation_id,
            kind: format!("{:?}", event.kind),
            public_message: event.public_message,
        }
    }
}

#[cfg(unix)]
fn set_socket_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_socket_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_socket_path_ends_with_control_sock() {
        assert!(default_socket_path().ends_with("control.sock"));
    }

    #[tokio::test]
    async fn rejects_unsupported_protocol_version() {
        let control = Arc::new(Mutex::new(ControlService::new()));
        let response = handle_envelope(
            ControlEnvelope {
                version: 99,
                request_id: "req_test".into(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::Status,
            },
            control,
        )
        .await;

        assert!(matches!(
            response,
            RpcControlResponse::Rejected { code, .. } if code == "unsupported_version"
        ));
    }
}
