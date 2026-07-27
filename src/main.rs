use clap::Parser;

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[arg(long, default_value = "./config.toml")]
    config: String,

    /// Send a startup "sebas 已启动" message to this chat_id, then continue running.
    /// Useful for verifying outbound is wired correctly. chat_id format depends on
    /// receive_id_type: open_id (private) or chat_id (group).
    #[arg(long)]
    test_msg: Option<String>,

    /// Dump every raw inbound WS payload to this directory as one .json file per
    /// event (timestamp-prefixed). Useful for local replay/debug without needing
    /// the live Feishu connection. Disabled when omitted.
    #[arg(long)]
    dump_inbound: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let raw = std::fs::read_to_string(&cli.config).unwrap_or_else(|_| {
        // No file -> use minimal defaults; require env vars for credentials
        let app_id = std::env::var("SEBAS_FEISHU_APP_ID").unwrap_or_default();
        let app_secret = std::env::var("SEBAS_FEISHU_APP_SECRET").unwrap_or_default();
        format!(
            "[feishu]\napp_id = \"{app_id}\"\napp_secret = \"{app_secret}\"\nowner_id = \"ou_xxx\"\n"
        )
    });
    let cfg = sebas::config::Config::parse(&raw).map_err(|e| anyhow::anyhow!("{e}"))?;
    sebas::run::run(cfg, cli.test_msg, cli.dump_inbound)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}
