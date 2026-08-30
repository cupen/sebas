//! `sebas gateway --config <path>` — LLM provider gateway entry.
//!
//! Reads the `[gateway]` config (TOML + env override + validate), builds the
//! server state, and runs the axum server with graceful shutdown. The gateway
//! crate owns the HTTP surface; this module is the root-binary adapter that
//! turns a `SebasError`-typed CLI call into `sebas_gateway::server::run`.
//!
//! See openspec/specs/gateway-core/spec.md.

use sebas_gateway::config::GatewayConfig;
use sebas_gateway::debug;
use sebas_gateway::server;

use crate::error::{Result, SebasError};

/// Arguments for `sebas gateway --config <path>`.
pub struct GatewayArgs {
    pub config: String,
    /// debug 模式：内置 test 模型，gateway 自身应答。
    pub debug: bool,
}

/// CLI entry: read + parse the gateway config, then run the server.
pub async fn run(args: GatewayArgs) -> Result<()> {
    init_tracing();
    let raw = std::fs::read_to_string(&args.config)
        .map_err(|e| SebasError::Gateway(format!("read config {}: {e}", args.config)))?;
    let mut cfg = GatewayConfig::parse(&raw).map_err(|e| SebasError::Gateway(e.to_string()))?;
    if args.debug {
        // parse 完成后注入内置 test provider（不改变配置解析语义）。
        // 实现在 `sebas_gateway::debug`。
        debug::enable_debug_test_provider(&mut cfg);
    }
    server::run(cfg)
        .await
        .map_err(|e| SebasError::Gateway(e.to_string()))?;
    Ok(())
}

/// Install a `tracing_subscriber` for the gateway path. `GatewayConfig` has no
/// `[log]` section, so the filter comes from `RUST_LOG` (default `"info"`),
/// mirroring `run.rs::init_tracing` minus the log-file writer. `try_init`
/// returns `Err` if a global subscriber is already installed — e.g. when a test
/// sets one up first — so the error is ignored: the first caller wins and
/// later calls are a no-op.
fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}
