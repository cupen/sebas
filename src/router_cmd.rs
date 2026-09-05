//! `sebas router --config <path>` — LLM provider router entry.
//!
//! Reads the `[router]` config (TOML + env override + validate), builds the
//! server state, and runs the axum server with graceful shutdown. The router
//! crate owns the HTTP surface; this module is the root-binary adapter that
//! turns a `SebasError`-typed CLI call into `sebas_router::server::run`.
//!
//! See openspec/specs/router-core/spec.md.

use sebas_router::config::RouterConfig;
use sebas_router::debug;
use sebas_router::server;

use crate::error::{Result, SebasError};

/// Arguments for `sebas router --config <path>`.
pub struct RouterArgs {
    pub config: String,
    /// debug 模式：内置 test 模型，router 自身应答。
    pub debug: bool,
}

/// CLI entry: read + parse the router config, then run the server.
pub async fn run(args: RouterArgs) -> Result<()> {
    init_tracing();
    let raw = std::fs::read_to_string(&args.config)
        .map_err(|e| SebasError::Router(format!("read config {}: {e}", args.config)))?;
    let mut cfg = RouterConfig::parse(&raw).map_err(|e| SebasError::Router(e.to_string()))?;
    if args.debug {
        // parse 完成后注入内置 test provider（不改变配置解析语义）。
        // 实现在 `sebas_router::debug`。
        debug::enable_debug_test_provider(&mut cfg);
    }
    server::run(cfg)
        .await
        .map_err(|e| SebasError::Router(e.to_string()))?;
    Ok(())
}

/// Install a `tracing_subscriber` for the router path. `RouterConfig` has no
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
