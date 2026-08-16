use crate::error::{Result, SebasError};
use crate::watchdog::control::{
    ControlEvent, ControlResponse, ControlRequest, ControlService, UpdateKind,
};
use crate::watchdog::executor::ControlExecutor;
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
    #[serde(default)]
    pub secret: String,
    pub actor: RpcActor,
    pub request: RpcControlRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcActor {
    Cli { uid: u32 },
    /// Feishu proxy actor (spec §6.2, Phase 3 core-hosted). The core submits
    /// this with the startup secret as the signed-assertion MAC basis; the
    /// watchdog maps it to `crate::watchdog::control::Actor::Feishu`.
    /// `open_id` is the Feishu sender; `chat_id` the originating chat.
    /// Pre-Phase-5 the core does not yet carry the sender's open_id on every
    /// inbound, so `chat_id` is authoritative and `open_id` may be empty.
    Feishu {
        open_id: String,
        chat_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum RpcControlRequest {
    Status,
    EventsSince { seq: u64 },
    Update { dev: bool, dry_run: bool },
    Rollback { dry_run: bool },
    RestartCore,
    ServiceStatus,
    /// \`/gateway on|off|status\` / \`/webui status\`: set a managed
    /// service's desired state. `service` ∈ {gateway, webui}, `desired` ∈
    /// {on, off}. Serve side returns `service_unavailable` until Phase 4
    /// (ServiceManager) lands.
    ServiceSet {
        service: String,
        desired: String,
        persist: bool,
    },
    /// \`/gateway restart\`: restart a managed service. Same Phase 4
    /// limitation as `ServiceSet`.
    ServiceRestart { service: String },
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
    Services {
        services: Vec<RpcServiceStatus>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcServiceStatus {
    pub name: String,
    pub status: String,
    pub desired: String,
    pub uptime_secs: Option<u64>,
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
    // XDG_RUNTIME_DIR unset (common in containers / non-login shells): fall
    // back to a per-user temp dir so multiple users on the same host do not
    // stomp on each other. Always end in `control.sock` so clients can rely
    // on the suffix.
    let base = std::env::temp_dir().join("sebas");
    if let Some(uid) = users_uid() {
        base.join(format!("uid{uid}")).join("control.sock")
    } else {
        base.join("control.sock")
    }
}

#[cfg(unix)]
fn users_uid() -> Option<u32> {
    // nix crate is not a dependency, so use libc directly to avoid pulling a
    // new crate just for getuid(). Already used elsewhere in this crate.
    unsafe { Some(libc::getuid()) }
}

#[cfg(not(unix))]
fn users_uid() -> Option<u32> {
    None
}

pub async fn serve(path: PathBuf, secret: String, executor: ControlExecutor) -> Result<()> {
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
        let executor = executor.clone();
        let secret = secret.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_stream(stream, executor, secret).await {
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

async fn handle_stream(stream: UnixStream, executor: ControlExecutor, secret: String) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    if line.is_empty() {
        return Ok(());
    }

    let response = match serde_json::from_str::<ControlEnvelope>(&line) {
        Ok(envelope) => handle_envelope(envelope, executor, &secret).await,
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
    executor: ControlExecutor,
    server_secret: &str,
) -> RpcControlResponse {
    if envelope.version != 1 {
        return RpcControlResponse::Rejected {
            code: "unsupported_version".into(),
            message: format!("unsupported control RPC version {}", envelope.version),
        };
    }

    if envelope.secret.is_empty() || envelope.secret != server_secret {
        return RpcControlResponse::Rejected {
            code: "unauthorized".into(),
            message: "missing or invalid control RPC secret".into(),
        };
    }

    match envelope.request {
        RpcControlRequest::Status => {
            accept_control_request(executor.control().clone(), envelope.actor, ControlRequest::Status).await
        }
        RpcControlRequest::EventsSince { seq } => {
            let control = executor.control().lock().await;
            RpcControlResponse::Events {
                events: control
                    .events_since(seq)
                    .into_iter()
                    .map(RpcControlEvent::from)
                    .collect(),
            }
        }
        RpcControlRequest::Update { dev, dry_run } => {
            // Feishu/CLI actors both map via From<RpcActor> for Actor.
            let actor = crate::watchdog::control::Actor::from(envelope.actor);
            let request = ControlRequest::Update {
                kind: if dev { UpdateKind::Dev } else { UpdateKind::Release },
                dry_run,
                target: None,
            };
            executor.submit_detached(actor, request).await.into()
        }
        RpcControlRequest::Rollback { dry_run } => {
            let actor = crate::watchdog::control::Actor::from(envelope.actor);
            executor.submit_detached(actor, ControlRequest::Rollback { dry_run }).await.into()
        }
        RpcControlRequest::RestartCore => {
            let actor = crate::watchdog::control::Actor::from(envelope.actor);
            executor.submit_detached(actor, ControlRequest::RestartCore).await.into()
        }
        RpcControlRequest::ServiceStatus => {
            executor.service_status().await
        }
        // Phase 3 (Task 3.1) only routes the command; the ServiceManager that
        // actually applies desired-state / restart lands in Phase 4. Until
        // then, be explicit rather than silently no-op.
        RpcControlRequest::ServiceSet {
            service,
            desired,
            persist: _,
        } => RpcControlResponse::Rejected {
            code: "service_unavailable".into(),
            message: format!(
                "service management not yet wired (Phase 4); received set {service}={desired}"
            ),
        },
        RpcControlRequest::ServiceRestart { service } => RpcControlResponse::Rejected {
            code: "service_unavailable".into(),
            message: format!(
                "service management not yet wired (Phase 4); received restart {service}"
            ),
        },
    }
}

async fn accept_control_request(
    control: Arc<Mutex<ControlService>>,
    actor: RpcActor,
    request: ControlRequest,
) -> RpcControlResponse {
    let mut control = control.lock().await;
    match control.accept(actor.into(), request) {
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

impl From<ControlResponse> for RpcControlResponse {
    fn from(r: ControlResponse) -> Self {
        match r {
            ControlResponse::Accepted { operation_id, status } => RpcControlResponse::Accepted {
                operation_id,
                status: format!("{status:?}"),
            },
            ControlResponse::Rejected { code, message } => RpcControlResponse::Rejected {
                code: format!("{code:?}"),
                message,
            },
        }
    }
}

impl From<RpcActor> for crate::watchdog::control::Actor {
    fn from(a: RpcActor) -> Self {
        match a {
            RpcActor::Cli { uid } => crate::watchdog::control::Actor::Cli { uid },
            RpcActor::Feishu { open_id, chat_id } => {
                crate::watchdog::control::Actor::Feishu { open_id, chat_id }
            }
        }
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
    use crate::config::WatchdogConfig;
    use crate::watchdog::updater::{UpdatePlan, UpdaterRunner};

    const TEST_SECRET: &str = "test-secret-42";

    struct NoopRunner;

    #[async_trait::async_trait]
    impl UpdaterRunner for NoopRunner {
        async fn run(&self, _plan: &UpdatePlan, _watchdog: &WatchdogConfig) -> Result<()> {
            Ok(())
        }
    }

    fn test_executor() -> ControlExecutor {
        let control = Arc::new(Mutex::new(ControlService::new()));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        ControlExecutor::new(
            control,
            Arc::new(NoopRunner),
            WatchdogConfig::default(),
            "./config.toml".into(),
            tx,
        )
    }

    #[test]
    fn default_socket_path_ends_with_control_sock() {
        assert!(default_socket_path().ends_with("control.sock"));
    }

    #[test]
    fn forged_system_actor_rejected() {
        let rpc = RpcActor::Cli { uid: 42 };
        let actor: crate::watchdog::control::Actor = rpc.into();
        assert!(matches!(actor, crate::watchdog::control::Actor::Cli { uid: 42 }));
    }

    #[tokio::test]
    async fn rejects_unsupported_protocol_version() {
        let response = handle_envelope(
            ControlEnvelope {
                version: 99,
                request_id: "req_test".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::Status,
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;

        assert!(matches!(
            response,
            RpcControlResponse::Rejected { code, .. } if code == "unsupported_version"
        ));
    }

    #[tokio::test]
    async fn missing_secret_rejected() {
        let response = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "test".into(),
                secret: String::new(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::Status,
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;

        assert!(matches!(
            response,
            RpcControlResponse::Rejected { code, .. } if code == "unauthorized"
        ));
    }

    #[tokio::test]
    async fn invalid_secret_rejected() {
        let response = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "test".into(),
                secret: "wrong-secret".into(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::Status,
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;

        assert!(matches!(
            response,
            RpcControlResponse::Rejected { code, .. } if code == "unauthorized"
        ));
    }

    #[tokio::test]
    async fn valid_secret_allows_status() {
        let response = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "test".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::Status,
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;

        assert!(
            !matches!(
                response,
                RpcControlResponse::Rejected { ref code, .. } if code == "unauthorized"
            ),
            "valid secret must not be rejected as unauthorized, got {response:?}"
        );
        assert!(
            matches!(response, RpcControlResponse::Accepted { .. }),
            "expected Accepted for Status with valid secret, got {response:?}"
        );
    }

    /// Feishu proxy actor (spec §6.2 Phase 3) maps to the watchdog Actor
    /// preserving open_id/chat_id, so downstream authorization can act on the
    /// real chat while the sender open_id is still absent pre-Phase-5.
    #[test]
    fn feishu_actor_maps_to_watchdog_actor() {
        let rpc = RpcActor::Feishu {
            open_id: String::new(),
            chat_id: Some("oc_abc".into()),
        };
        let actor: crate::watchdog::control::Actor = rpc.into();
        assert!(matches!(
            actor,
            crate::watchdog::control::Actor::Feishu {
                ref open_id,
                ref chat_id,
            } if open_id.is_empty() && chat_id.as_deref() == Some("oc_abc")
        ));
    }

    /// Feishu actor with a valid secret may query Status: the new actor path
    /// must not regress the authenticated control plane.
    #[tokio::test]
    async fn feishu_actor_with_valid_secret_allows_status() {
        let response = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "feishu_status".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Feishu {
                    open_id: String::new(),
                    chat_id: Some("oc_abc".into()),
                },
                request: RpcControlRequest::Status,
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;

        assert!(
            matches!(response, RpcControlResponse::Accepted { .. }),
            "expected Accepted for Feishu Status, got {response:?}"
        );
    }

    /// ServiceSet is routed but rejected until Phase 4 (ServiceManager) lands;
    /// be explicit rather than silently no-op (plan Task 3.1).
    #[tokio::test]
    async fn service_set_rejected_until_phase_4() {
        let response = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "feishu_gateway".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Feishu {
                    open_id: String::new(),
                    chat_id: Some("oc_abc".into()),
                },
                request: RpcControlRequest::ServiceSet {
                    service: "gateway".into(),
                    desired: "on".into(),
                    persist: false,
                },
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;

        assert!(matches!(
            response,
            RpcControlResponse::Rejected { ref code, .. } if code == "service_unavailable"
        ));
    }

    #[tokio::test]
    async fn service_restart_rejected_until_phase_4() {
        let response = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "feishu_gateway_restart".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Feishu {
                    open_id: String::new(),
                    chat_id: Some("oc_abc".into()),
                },
                request: RpcControlRequest::ServiceRestart {
                    service: "gateway".into(),
                },
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;

        assert!(matches!(
            response,
            RpcControlResponse::Rejected { ref code, .. } if code == "service_unavailable"
        ));
    }
}
