//! Standalone WebUI server entry point.
//!
//! Spawned by the watchdog as a separate process when `[watchdog.webui] enabled`
//! is `true`. Runs independently of the core child so the dashboard stays up
//! across core restarts (`sebas watchdog` restarts the core child; the WebUI
//! process is unaffected).
//!
//! # Limitations
//!
//! The standalone WebUI creates its own [`RouterHandle`] and [`SessionManager`]
//! from the state file on disk. It does **not** share live session state with
//! the core child:
//!
//! - Session data is as fresh as the last state file dump (written by the core
//!   child on shutdown and after significant state changes).
//! - API endpoints that create sessions (`POST /api/sessions`) or send messages
//!   (`POST /api/sessions/{key}/message`) will create router entries but will
//!   **not** create actual ACP sessions, since the WebUI process has no running
//!   ACP manager.
//! - Session close (`POST /api/sessions/{key}/close`) will remove the mapping
//!   from the local router but will not affect any running child processes.
//!
//! Phase 2.3+ should introduce an IPC bridge between the WebUI process and the
//! core child, or fold the WebUI back into the core child with watchdog-controlled
//! lifecycle commands.

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
use webui::admin::{
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
    let merged_card_cfg = load_card_config(&cfg);

    // Restore session map from the state file (same path as the core child).
    let map =
        crate::run::restore_session_map(&cfg.router.state_file, cfg.router.max_concurrent_sessions);

    // Create a throwaway session manager. No actual ACP sessions run in this
    // process, so the manager is a stub that accepts calls but has no children.
    let mgr = Arc::new(acp_claude::manager::SessionManager::new(
        std::time::Duration::from_secs(cfg.acp.claude.startup_timeout_secs),
    ));

    // Create the router. The outbound rx is dropped intentionally: the
    // standalone WebUI has no outbound dispatch pump, so `Out` instructions
    // (session creation, message sending, card updates) are silently dropped.
    let (router, _out_rx) = router::router::RouterHandle::new_with_config(
        map,
        merged_card_cfg,
        cfg.router.channel_buffer,
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
    webui::run_with_admin_adapter(
        router,
        mgr,
        webui::models::GatewayInfo::default(),
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
fn load_card_config(cfg: &Config) -> feishu::cards::CardConfig {
    match router::settings::load_settings(&router::settings::settings_path()) {
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
