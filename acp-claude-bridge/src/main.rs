mod claude;
mod permission;
mod server;
mod translator;

use claude::driver::ClaudeDriver;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // For Task 8: just verify the server wires up. The actual driver + broker
    // are wired in Task 9.
    let _claude: ClaudeDriver = ClaudeDriver::spawn("true", &["/dev/null"]).await?;
    let (_broker, perm_rx) = permission::PermissionBroker::bind().await?;
    server::run(_claude, perm_rx).await
}
