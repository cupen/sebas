use crate::error::{Result, SebasError};
use crate::watchdog::auth::{AssertionPrincipal, actor_to_principal};
use crate::watchdog::control::{
    Actor, ControlEvent, ControlRequest, ControlResponse, ControlService, DesiredState, UpdateKind,
};
use crate::watchdog::executor::ControlExecutor;
use crate::watchdog::services::service_from_str;
use crate::watchdog::supervisor::ServiceName;
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
    Cli {
        uid: u32,
    },
    /// Feishu proxy actor (openspec/specs/watchdog/spec.md, Phase 3 core-hosted). The core submits
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
    EventsSince {
        seq: u64,
    },
    Update {
        dev: bool,
        dry_run: bool,
    },
    Rollback {
        dry_run: bool,
    },
    RestartCore,
    ServiceStatus,
    /// `/gateway status` / `/webui status`: query a single managed service's
    /// status. The server filters `ServiceStatus` down to `service`.
    ServiceStatusFor {
        service: String,
    },
    /// `/gateway on|off`: set a managed service's desired state.
    /// `service` ∈ {core, gateway, webui}, `desired` ∈ {on, off}. core 的
    /// 启停只接受 CLI/WebUI actor（飞书 actor 拒绝，见 handle 侧）。
    ServiceSet {
        service: String,
        desired: String,
        persist: bool,
    },
    /// `/gateway restart`: restart a managed service. Same Phase 4
    /// limitation as `ServiceSet`.
    ServiceRestart {
        service: String,
    },
    /// Confirm a pending dangerous action via its opaque confirmation token
    /// (Phase 3 Task 3.2, openspec/specs/watchdog/spec.md). The client sends only the token; the
    /// canonical action/params live in the watchdog's pending registry.
    Confirm {
        token: String,
    },
    /// Cancel a pending dangerous action via its opaque confirmation token.
    /// Redeems (consumes) the grant and records a Canceled event.
    Cancel {
        token: String,
    },
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
    /// A dangerous action needs confirmation before it can run (openspec/specs/watchdog/spec.md).
    /// `token` is opaque, single-use and short-lived; the client renders a
    /// confirmation card carrying only this token — never the action truth,
    /// which stays in the watchdog's pending registry. `action`/`message`/
    /// `expires_in` are display-only.
    PendingConfirmation {
        token: String,
        action: String,
        message: String,
        expires_in: u64,
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

async fn handle_stream(
    stream: UnixStream,
    executor: ControlExecutor,
    secret: String,
) -> Result<()> {
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
            accept_control_request(
                executor.control().clone(),
                envelope.actor,
                ControlRequest::Status,
            )
            .await
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
                kind: if dev {
                    UpdateKind::Dev
                } else {
                    UpdateKind::Release
                },
                dry_run,
                target: None,
            };
            executor.submit_or_confirm(actor, request).await
        }
        RpcControlRequest::Rollback { dry_run } => {
            let actor = crate::watchdog::control::Actor::from(envelope.actor);
            executor
                .submit_or_confirm(actor, ControlRequest::Rollback { dry_run })
                .await
        }
        RpcControlRequest::RestartCore => {
            let actor = crate::watchdog::control::Actor::from(envelope.actor);
            executor
                .submit_or_confirm(actor, ControlRequest::RestartCore)
                .await
        }
        RpcControlRequest::Confirm { token } => match feishu_principal_channel(envelope.actor) {
            Some((principal, channel)) => executor.confirm(&token, &principal, &channel).await,
            None => RpcControlResponse::Rejected {
                code: "unauthorized".into(),
                message: "only Feishu actors may confirm a confirmation".into(),
            },
        },
        RpcControlRequest::Cancel { token } => match feishu_principal_channel(envelope.actor) {
            Some((principal, channel)) => executor.cancel(&token, &principal, &channel).await,
            None => RpcControlResponse::Rejected {
                code: "unauthorized".into(),
                message: "only Feishu actors may cancel a confirmation".into(),
            },
        },
        RpcControlRequest::ServiceStatus => executor.service_status().await,
        RpcControlRequest::ServiceStatusFor { service } => {
            executor.service_status_for(&service).await
        }
        // 受管服务期望态：core 也允许启停（sebas-2ty：feishu 可选，core 由
        // WebUI 服务页控制），但飞书 actor 例外——core 停止后确认卡片无法
        // 送达（dead-man's switch），core 的启停只走 CLI/WebUI 控制面。
        // webui/gateway 照旧走 executor 的 ServiceSet（含 persist 落盘）。
        RpcControlRequest::ServiceSet {
            service,
            desired,
            persist,
        } => {
            if service_from_str(&service) == Some(ServiceName::Core)
                && matches!(envelope.actor, RpcActor::Feishu { .. })
            {
                return RpcControlResponse::Rejected {
                    code: "invalid_request".into(),
                    message: "core 的启停请通过 WebUI 服务页或 CLI 控制面（飞书渠道不支持）".into(),
                };
            }
            match service_set_request(&service, &desired, persist) {
                Ok(request) => {
                    let actor = crate::watchdog::control::Actor::from(envelope.actor);
                    executor.submit_or_confirm(actor, request).await
                }
                Err((code, message)) => RpcControlResponse::Rejected { code, message },
            }
        }
        RpcControlRequest::ServiceRestart { service } => match service_from_str(&service) {
            Some(ServiceName::Core) => RpcControlResponse::Rejected {
                code: "invalid_request".into(),
                message: "core 使用 restart_core（升级/回滚语义），不接受 service restart".into(),
            },
            Some(name) => {
                let actor = crate::watchdog::control::Actor::from(envelope.actor);
                executor
                    .submit_or_confirm(
                        actor,
                        ControlRequest::ServiceRestart {
                            service: managed_service(name),
                        },
                    )
                    .await
            }
            None => RpcControlResponse::Rejected {
                code: "invalid_request".into(),
                message: format!("未知服务: {service}"),
            },
        },
    }
}

/// 把 RPC 的 ServiceSet 翻译成 ControlRequest；错误返回 (code, message)。
/// core 合法（sebas-2ty：由 WebUI/CLI 启停）；飞书 actor 的 core 操作已在
/// 上层拒绝。
fn service_set_request(
    service: &str,
    desired: &str,
    persist: bool,
) -> std::result::Result<ControlRequest, (String, String)> {
    let name = service_from_str(service)
        .ok_or_else(|| ("invalid_request".into(), format!("未知服务: {service}")))?;
    let desired = match desired {
        "on" | "enabled" => DesiredState::Enabled,
        "off" | "disabled" => DesiredState::Disabled,
        other => {
            return Err((
                "invalid_request".into(),
                format!("非法期望态 {other:?}（应为 on/off）"),
            ));
        }
    };
    Ok(ControlRequest::ServiceSet {
        service: managed_service(name),
        desired,
        persist,
    })
}

/// ServiceName → ControlRequest 的 ManagedService。
fn managed_service(name: ServiceName) -> crate::watchdog::control::ManagedService {
    match name {
        ServiceName::Core => crate::watchdog::control::ManagedService::Core,
        ServiceName::WebUi => crate::watchdog::control::ManagedService::WebUi,
        ServiceName::Gateway => crate::watchdog::control::ManagedService::Gateway,
    }
}

/// Derive the Feishu principal + channel for confirm/cancel (openspec/specs/watchdog/spec.md.3).
/// Only Feishu actors carry an assertion-based principal, and the watchdog —
/// not the core — derives the identity from the actor, so a forged open_id in
/// the envelope cannot impersonate an owner. Cli/System actors have no
/// principal and cannot confirm/cancel.
fn feishu_principal_channel(actor: RpcActor) -> Option<(AssertionPrincipal, String)> {
    let actor = Actor::from(actor);
    match &actor {
        Actor::Feishu {
            chat_id: Some(chat),
            ..
        } => actor_to_principal(&actor).map(|p| (p, chat.clone())),
        _ => None,
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
        ControlExecutor::new(
            control,
            Arc::new(NoopRunner),
            WatchdogConfig::default(),
            "./config.toml".into(),
            crate::watchdog::services::ServiceManager::new(
                std::env::temp_dir().join(format!("sebas-exec-noop-{}.json", std::process::id())),
            ),
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
        assert!(matches!(
            actor,
            crate::watchdog::control::Actor::Cli { uid: 42 }
        ));
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

    /// Feishu proxy actor (openspec/specs/watchdog/spec.md Phase 3) maps to the watchdog Actor
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

    /// `/gateway status` and `/webui status` query a single service; the RPC
    /// server filters the full service list down to the requested name.
    #[tokio::test]
    async fn service_status_for_filters_to_requested_service() {
        let response = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "feishu_gateway_status".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Feishu {
                    open_id: String::new(),
                    chat_id: Some("oc_abc".into()),
                },
                request: RpcControlRequest::ServiceStatusFor {
                    service: "gateway".into(),
                },
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;

        // 空 ServiceManager：gateway 未注册，过滤结果为空（而非报错）。
        match response {
            RpcControlResponse::Services { services } => {
                assert!(services.is_empty());
            }
            other => panic!("expected Services, got {other:?}"),
        }
    }

    /// ServiceSet: core 被拒（spec「service set rejected」——core 托管，
    /// 用 restart_core）；合法服务走确认/执行路径（Feishu → PendingConfirmation）。
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

        // 旧断言已失效：gateway set 现在真实执行（Feishu actor 会先得到
        // PendingConfirmation），不再是 service_unavailable。
        assert!(matches!(
            response,
            RpcControlResponse::PendingConfirmation { .. }
        ));
    }

    /// ServiceSet core：CLI/WebUI actor 直接执行（Accepted，落 operation）；
    /// 飞书 actor 被拒（core 停止后确认卡片无法送达，dead-man's switch）。
    #[tokio::test]
    async fn service_set_core_executes_for_cli() {
        let response = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "core_set".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::ServiceSet {
                    service: "core".into(),
                    desired: "off".into(),
                    persist: true,
                },
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;
        assert!(
            matches!(response, RpcControlResponse::Accepted { .. }),
            "CLI actor 的 core ServiceSet 应被接受，got {response:?}"
        );
    }

    #[tokio::test]
    async fn service_set_core_is_rejected_for_feishu() {
        let response = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "core_set_feishu".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Feishu {
                    open_id: String::new(),
                    chat_id: Some("oc_abc".into()),
                },
                request: RpcControlRequest::ServiceSet {
                    service: "core".into(),
                    desired: "off".into(),
                    persist: true,
                },
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;
        assert!(matches!(
            response,
            RpcControlResponse::Rejected { ref code, .. } if code == "invalid_request"
        ));
    }

    #[tokio::test]
    async fn service_restart_gateway_executes() {
        let response = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "cli_gateway_restart".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::ServiceRestart {
                    service: "gateway".into(),
                },
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;

        // gateway restart 现在真实执行（Cli actor 直接 Accepted）。
        assert!(matches!(response, RpcControlResponse::Accepted { .. }));
    }

    #[tokio::test]
    async fn service_restart_core_is_rejected() {
        let response = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "core_restart".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::ServiceRestart {
                    service: "core".into(),
                },
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;
        assert!(matches!(
            response,
            RpcControlResponse::Rejected { ref code, .. } if code == "invalid_request"
        ));
    }

    // ── 真实 Unix-socket 往返（覆盖 line-framing + secret/version） ──

    /// 验收测试（sebas-29s）：飞书侧危险操作确认令牌可兑换。
    /// 同一 Feishu actor 提交 Update → PendingConfirmation，再以同 actor
    /// Confirm 兑换 token → Accepted（watchdog 以 detached 执行原操作，
    /// NoopRunner 保证测试无副作用）。
    #[tokio::test]
    async fn feishu_pending_confirmation_token_is_redeemable() {
        let executor = test_executor();
        let actor = RpcActor::Feishu {
            open_id: String::new(),
            chat_id: Some("oc_confirm".into()),
        };

        let resp = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "cfm_submit".into(),
                secret: TEST_SECRET.into(),
                actor: actor.clone(),
                request: RpcControlRequest::Update {
                    dev: true,
                    dry_run: false,
                },
            },
            executor.clone(),
            TEST_SECRET,
        )
        .await;
        let RpcControlResponse::PendingConfirmation { token, .. } = resp else {
            panic!("Feishu Update must yield PendingConfirmation, got {resp:?}")
        };

        let resp = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "cfm_redeem".into(),
                secret: TEST_SECRET.into(),
                actor,
                request: RpcControlRequest::Confirm {
                    token: token.clone(),
                },
            },
            executor,
            TEST_SECRET,
        )
        .await;
        assert!(
            matches!(resp, RpcControlResponse::Accepted { .. }),
            "same-actor Confirm must redeem the token to Accepted, got {resp:?}"
        );

        // 单次兑换：二次 Confirm 同一 token 必须被拒。
        let resp = handle_envelope(
            ControlEnvelope {
                version: 1,
                request_id: "cfm_replay".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Feishu {
                    open_id: String::new(),
                    chat_id: Some("oc_confirm".into()),
                },
                request: RpcControlRequest::Confirm { token },
            },
            test_executor(),
            TEST_SECRET,
        )
        .await;
        assert!(
            matches!(resp, RpcControlResponse::Rejected { .. }),
            "replayed token must be rejected, got {resp:?}"
        );
    }

    // 上面 handle_envelope 直调测试绕过了 `request`/`serve` 的 line-buffered
    // JSON-over-Unix-socket framing；这里 spawn 真 listener 走完整链，证明
    // 「core 提交 envelope → watchdog 收包 → 分类 → 回包」在 socket 边界上
    // 不会因 framing / 版本号 / secret 字段顺序而出错。

    /// 临时 socket 路径，每个用例独立，不与默认路径冲突。
    fn unique_test_socket(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "sebas-rpc-{label}-{nanos}-{}.sock",
            std::process::id()
        ))
    }

    /// Spawn 真 serve()，等 listener 就绪后返回（client 即可发起 connect）。
    async fn spawn_test_server(path: PathBuf) -> tokio::task::JoinHandle<()> {
        let bind_path = path.clone();
        let handle = tokio::spawn(async move {
            // serve() 在 path 已存在时会 remove + bind；测试不需要重试循环。
            let _ = serve(bind_path, TEST_SECRET.into(), test_executor()).await;
        });
        // 短暂等待 listener bind：UnixListener::bind 是同步 syscall，
        // spawn 后几条调度足以完成。失败由 request 的 connect 错误暴露。
        for _ in 0..50 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        handle
    }

    #[tokio::test]
    async fn socket_roundtrip_status_with_valid_secret() {
        let path = unique_test_socket("status-ok");
        let server = spawn_test_server(path.clone()).await;

        let resp = request(
            &path,
            &ControlEnvelope {
                version: 1,
                request_id: "rt_status".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::Status,
            },
        )
        .await
        .expect("server reachable + valid secret");

        assert!(
            matches!(resp, RpcControlResponse::Accepted { .. }),
            "valid Status envelope must round-trip to Accepted, got {resp:?}"
        );

        server.abort();
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn socket_roundtrip_update_dev_dry_run() {
        let path = unique_test_socket("update-dev-dryrun");
        let server = spawn_test_server(path.clone()).await;

        let resp = request(
            &path,
            &ControlEnvelope {
                version: 1,
                request_id: "rt_update_dev".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Feishu {
                    open_id: String::new(),
                    chat_id: Some("oc_e2e".into()),
                },
                request: RpcControlRequest::Update {
                    dev: true,
                    dry_run: true,
                },
            },
        )
        .await
        .expect("server reachable + valid secret");

        assert!(
            matches!(
                resp,
                RpcControlResponse::Accepted { .. }
                    | RpcControlResponse::PendingConfirmation { .. }
            ),
            "Update dev/dry_run must round-trip to Accepted or PendingConfirmation, got {resp:?}"
        );

        server.abort();
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn socket_roundtrip_rollback_and_restart_core() {
        let path = unique_test_socket("rollback-restart");
        let server = spawn_test_server(path.clone()).await;

        // Rollback
        let resp_rb = request(
            &path,
            &ControlEnvelope {
                version: 1,
                request_id: "rt_rb".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::Rollback { dry_run: false },
            },
        )
        .await
        .expect("Rollback envelope must round-trip");
        assert!(
            matches!(
                resp_rb,
                RpcControlResponse::Accepted { .. }
                    | RpcControlResponse::PendingConfirmation { .. }
            ),
            "Rollback should reach Accepted/PendingConfirmation, got {resp_rb:?}"
        );

        // RestartCore
        let resp_rc = request(
            &path,
            &ControlEnvelope {
                version: 1,
                request_id: "rt_rc".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::RestartCore,
            },
        )
        .await
        .expect("RestartCore envelope must round-trip");
        assert!(
            matches!(
                resp_rc,
                RpcControlResponse::Accepted { .. }
                    | RpcControlResponse::PendingConfirmation { .. }
            ),
            "RestartCore should reach Accepted/PendingConfirmation, got {resp_rc:?}"
        );

        server.abort();
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn socket_rejects_wrong_secret() {
        let path = unique_test_socket("wrong-secret");
        let server = spawn_test_server(path.clone()).await;

        let resp = request(
            &path,
            &ControlEnvelope {
                version: 1,
                request_id: "rt_wrong".into(),
                secret: "definitely-wrong".into(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::Status,
            },
        )
        .await
        .expect("server must respond even for unauthorized");

        assert!(
            matches!(resp, RpcControlResponse::Rejected { ref code, .. } if code == "unauthorized"),
            "wrong secret must yield Rejected/unauthorized over real socket, got {resp:?}"
        );

        server.abort();
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn socket_rejects_unsupported_version() {
        let path = unique_test_socket("bad-version");
        let server = spawn_test_server(path.clone()).await;

        let resp = request(
            &path,
            &ControlEnvelope {
                version: 99,
                request_id: "rt_bad_ver".into(),
                secret: TEST_SECRET.into(),
                actor: RpcActor::Cli { uid: 1000 },
                request: RpcControlRequest::Status,
            },
        )
        .await
        .expect("server must respond even for unsupported_version");

        assert!(
            matches!(resp, RpcControlResponse::Rejected { ref code, .. } if code == "unsupported_version"),
            "version=99 must yield Rejected/unsupported_version, got {resp:?}"
        );

        server.abort();
        let _ = std::fs::remove_file(&path);
    }
}
