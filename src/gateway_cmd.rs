//! `sebas gateway --config <path>` — LLM provider gateway entry.
//!
//! Reads the `[gateway]` config (TOML + env override + validate), builds the
//! server state, and runs the axum server with graceful shutdown. The gateway
//! crate owns the HTTP surface; this module is the root-binary adapter that
//! turns a `SebasError`-typed CLI call into `gateway::server::run`.
//!
//! See docs/superpowers/specs/2026-08-06-gateway-design.md.

use gateway::config::GatewayConfig;
use gateway::server;

use crate::error::{Result, SebasError};

/// Arguments for `sebas gateway --config <path>`.
pub struct GatewayArgs {
    pub config: String,
}

/// CLI entry: read + parse the gateway config, then run the server.
pub async fn run(args: GatewayArgs) -> Result<()> {
    let raw = std::fs::read_to_string(&args.config)
        .map_err(|e| SebasError::Gateway(format!("read config {}: {e}", args.config)))?;
    let cfg = GatewayConfig::parse(&raw).map_err(|e| SebasError::Gateway(e.to_string()))?;
    server::run(cfg)
        .await
        .map_err(|e| SebasError::Gateway(e.to_string()))?;
    Ok(())
}
