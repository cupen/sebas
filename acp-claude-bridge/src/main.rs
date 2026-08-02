mod claude;
mod notifications;
mod permission;
mod server;
mod translator;

use claude::driver::ClaudeDriver;
use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        // 写 stderr：bridge 的 stdout 是 JSON-RPC 通道；写 stdout 会污染帧导致
        // acp-claude SDK 的 JSON parser 失败。
        .with_writer(std::io::stderr)
        .init();

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
