//! Standalone WebUI server entry point.
//!
//! Spawned by the watchdog as a separate process when `[watchdog.webui] enabled`
//! is `true`. Runs independently of the core child so the dashboard stays up
//! across core restarts (`sebas watchdog` restarts the core child; the WebUI
//! process is unaffected).
//!
//! # Session data via the core session channel
//!
//! The standalone WebUI is a pure **client** of the core session channel (a
//! Unix-socket NDJSON protocol served by the core child): session reads and
//! mutations go through the socket backend (`core_channel::client`), so every
//! page shows the core's live state and every control reaches the real
//! session authority. When the core is not running, the backend reports
//! unreachable with its cause and the console renders that honestly —
//! no control reports success.

use crate::config::Config;
use crate::error::{Result, SebasError};
use crate::watchdog::control_rpc::{
    self, ControlEnvelope, RpcActor, RpcControlRequest, RpcControlResponse,
};
use crate::watchdog::services::WebUiEndpoint;
use crate::watchdog::EXIT_BIND_FAILED;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};
use sebas_webui::admin::{
    AdminAdapter, AdminEvent, AdminMutationResult, AdminOperation, AdminService, AdminStatus,
};

/// Arguments for `sebas webui --config <path>`.
pub struct WebUiArgs {
    pub config: String,
}

impl WebUiArgs {
    pub fn new(config: String) -> Self {
        Self { config }
    }
}

/// CLI entry: read + parse the config, then run the standalone WebUI server.
pub async fn run(args: WebUiArgs) -> Result<()> {
    init_tracing();

    let raw = std::fs::read_to_string(&args.config)
        .map_err(|e| SebasError::Config(format!("read config {}: {e}", args.config)))?;
    let cfg = Config::parse(&raw)?;

    // Build the WebUI endpoint from config (enabled, host, port).
    // Returns None when watchdog.webui.enabled is false — we require it to be
    // true because the standalone WebUI is a watchdog-owned service.
    let endpoint = WebUiEndpoint::from_config(&cfg.watchdog.webui)
        .ok_or_else(|| SebasError::Config("watchdog.webui.enabled is false".into()))?;
    if !endpoint.is_loopback() {
        return Err(SebasError::Config(
            "watchdog.webui.host must be loopback until secure WebUI mode is configured".into(),
        ));
    }

    info!(
        "starting standalone webui on {} (config={})",
        endpoint.bind_addr(),
        args.config
    );

    // Load card config: settings.json wins if present, else TOML `[card]`.
    // (The session channel does not transport settings; the settings page
    // renders this local snapshot.)
    let merged_card_cfg = load_card_config(&cfg);

    // The session backend: a client of the core session channel. The core
    // child owns the sessions; this process only renders and forwards.
    let backend = crate::core_channel::client::CoreChannelBackend::new(
        crate::core_channel::socket_path(&cfg),
        std::env::var("SEBAS_CORE_SECRET").ok().unwrap_or_default(),
    );

    // Bind to the configured port. Fails if the port is already in use
    // (by another WebUI process or the legacy `sebas run --webui` path).
    // On failure, exit with a specific code so the watchdog supervisor can
    // distinguish bind failures from other crashes and mark the service as
    // Degraded instead of endlessly retrying.
    let listener = match tokio::net::TcpListener::bind(endpoint.bind_addr()).await {
        Ok(l) => l,
        Err(e) => {
            warn!(
                "bind webui {} failed: {e}; exiting with code {} (Degraded)",
                endpoint.bind_addr(),
                EXIT_BIND_FAILED,
            );
            std::process::exit(EXIT_BIND_FAILED);
        }
    };

    let admin_adapter = control_admin_adapter();

    info!("webui dashboard listening on {}", endpoint.bind_addr());

    // Run the WebUI server. This blocks until the server stops.
    let backend_dyn: Arc<dyn sebas_webui::SessionBackend> = backend;
    sebas_webui::run_with_admin_adapter(
        backend_dyn,
        sebas_webui::models::GatewayInfo::default(),
        merged_card_cfg,
        listener,
        admin_adapter,
    )
    .await;

    info!("webui dashboard stopped");
    Ok(())
}

fn control_admin_adapter() -> Option<Arc<dyn AdminAdapter>> {
    let secret = match std::env::var("SEBAS_CONTROL_SECRET") {
        Ok(secret) if !secret.is_empty() => secret,
        _ => {
            warn!("SEBAS_CONTROL_SECRET not set; admin control routes are read-only");
            return None;
        }
    };
    Some(Arc::new(ControlRpcAdminAdapter {
        socket_path: control_rpc::default_socket_path(),
        secret,
    }))
}

struct ControlRpcAdminAdapter {
    socket_path: PathBuf,
    secret: String,
}

impl ControlRpcAdminAdapter {
    async fn send_request(&self, request: RpcControlRequest) -> Result<RpcControlResponse> {
        control_rpc::request(
            &self.socket_path,
            &ControlEnvelope {
                version: 1,
                request_id: "webui_admin".into(),
                secret: self.secret.clone(),
                actor: RpcActor::Cli { uid: current_uid() },
                request,
            },
        )
        .await
    }

    async fn submit(
        &self,
        request: RpcControlRequest,
        message: impl Into<String>,
    ) -> std::result::Result<AdminMutationResult, String> {
        match self.send_request(request).await {
            Ok(RpcControlResponse::Accepted {
                operation_id,
                status,
            }) => Ok(AdminMutationResult {
                operation_id,
                status,
                message: message.into(),
            }),
            Ok(RpcControlResponse::Rejected { code, message }) => {
                Err(format!("rejected [{code}]: {message}"))
            }
            Ok(other) => Err(format!("unexpected response: {other:?}")),
            Err(e) => Err(format!("control RPC failed: {e}")),
        }
    }
}

#[async_trait]
impl AdminAdapter for ControlRpcAdminAdapter {
    async fn status(&self) -> std::result::Result<AdminStatus, String> {
        match self.send_request(RpcControlRequest::Status).await {
            Ok(RpcControlResponse::Accepted {
                operation_id,
                status,
            }) => {
                let operation = AdminOperation {
                    operation_id,
                    request_type: "status".into(),
                    status,
                    message: "control RPC connected".into(),
                };
                Ok(AdminStatus {
                    version: env!("CARGO_PKG_VERSION").into(),
                    uptime_secs: 0,
                    operations: vec![operation.clone()],
                    active_operation: Some(operation),
                })
            }
            Ok(RpcControlResponse::Rejected { code, message }) => {
                Err(format!("rejected [{code}]: {message}"))
            }
            Ok(other) => Err(format!("unexpected response: {other:?}")),
            Err(e) => Err(format!("control RPC failed: {e}")),
        }
    }

    async fn events_since(&self, seq: u64) -> std::result::Result<Vec<AdminEvent>, String> {
        match self
            .send_request(RpcControlRequest::EventsSince { seq })
            .await
        {
            Ok(RpcControlResponse::Events { events }) => Ok(events
                .into_iter()
                .map(|e| AdminEvent {
                    seq: e.seq,
                    operation_id: e.operation_id,
                    kind: e.kind,
                    message: e.public_message,
                })
                .collect()),
            Ok(RpcControlResponse::Rejected { code, message }) => {
                Err(format!("rejected [{code}]: {message}"))
            }
            Ok(other) => Err(format!("unexpected response: {other:?}")),
            Err(e) => Err(format!("control RPC failed: {e}")),
        }
    }

    async fn service_set(
        &self,
        service: &str,
        desired: &str,
    ) -> std::result::Result<AdminMutationResult, String> {
        self.submit(
            RpcControlRequest::ServiceSet {
                service: service.into(),
                desired: desired.into(),
                // WebUI 服务页的启停选择持久化：watchdog 重启后保持用户意图。
                persist: true,
            },
            format!("service {service} set to {desired}"),
        )
        .await
    }

    async fn update(
        &self,
        dev: bool,
        dry_run: bool,
    ) -> std::result::Result<AdminMutationResult, String> {
        self.submit(
            RpcControlRequest::Update { dev, dry_run },
            format!("update accepted (dev={dev}, dry_run={dry_run})"),
        )
        .await
    }

    async fn rollback(&self, dry_run: bool) -> std::result::Result<AdminMutationResult, String> {
        self.submit(
            RpcControlRequest::Rollback { dry_run },
            format!("rollback accepted (dry_run={dry_run})"),
        )
        .await
    }

    async fn restart_core(&self) -> std::result::Result<AdminMutationResult, String> {
        self.submit(RpcControlRequest::RestartCore, "restart core accepted")
            .await
    }

    async fn services(&self) -> std::result::Result<Vec<AdminService>, String> {
        match self.send_request(RpcControlRequest::ServiceStatus).await {
            Ok(RpcControlResponse::Services { services }) => Ok(services
                .into_iter()
                .map(|s| AdminService {
                    name: s.name,
                    status: s.status,
                    desired: s.desired,
                    uptime_secs: s.uptime_secs,
                })
                .collect()),
            Ok(RpcControlResponse::Rejected { code, message }) => {
                Err(format!("rejected [{code}]: {message}"))
            }
            Ok(other) => Err(format!("unexpected response: {other:?}")),
            Err(e) => Err(format!("control RPC failed: {e}")),
        }
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

/// Load card config from settings.json, falling back to the TOML `[card]` section.
fn load_card_config(cfg: &Config) -> sebas_feishu::cards::CardConfig {
    match sebas_router::settings::load_settings(&sebas_router::settings::settings_path()) {
        Ok(Some(s)) => s,
        Ok(None) => cfg.card.clone(),
        Err(e) => {
            warn!(error = %e, "settings.json parse failed; using config defaults");
            cfg.card.clone()
        }
    }
}

/// Install a tracing subscriber for the standalone WebUI process.
/// Filter comes from `RUST_LOG` (default `"info"`), mirroring gateway_cmd.
/// `try_init` is used so the first caller wins and later calls are no-ops.
fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}
