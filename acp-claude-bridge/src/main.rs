mod claude;
mod notifications;
mod permission;
mod server;
mod translator;

use claude::driver::ClaudeDriver;
use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // bridge 自己的 stdout 是 JSON-RPC 通道，stderr 默认被 agent-client-protocol SDK
    // 通过 pipe 接管（sebas 端抓不到）。诊断模式下设 SEBAS_BRIDGE_LOG=/path/to.log
    // 直接落盘，避开 SDK 的 stdio 管理；未设则走 stderr（默认）。
    let bridge_log = env::var("SEBAS_BRIDGE_LOG").ok();
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    if let Some(path) = bridge_log.as_deref() {
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(file)
                .init();
        } else {
            // Fall back to stderr if the file can't be opened; better than silent.
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(std::io::stderr)
                .init();
        }
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            // 写 stderr：bridge 的 stdout 是 JSON-RPC 通道；写 stdout 会污染帧导致
            // acp-claude SDK 的 JSON parser 失败。
            .with_writer(std::io::stderr)
            .init();
    }

    // Args: <path-to-claude> [claude-args...]
    // In production, sebas's acp-claude spawns this binary with no args; the
    // path to claude is read from the env var SEBAS_CLAUDE_PATH (set by
    // acp-claude) with a fallback to "claude" on PATH.
    let claude_path = env::var("SEBAS_CLAUDE_PATH").unwrap_or_else(|_| "claude".into());
    let extra: Vec<String> = env::args().skip(1).collect();
    let extra_refs: Vec<&str> = extra.iter().map(String::as_str).collect();

    let claude = ClaudeDriver::spawn(&claude_path, &extra_refs).await?;
    let (broker, perm_tx) = permission::PermissionBroker::bind().await?;
    tokio::spawn(broker.run());
    server::run(claude, perm_tx).await
}
