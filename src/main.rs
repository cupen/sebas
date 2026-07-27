use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

/// Default mode (no subcommand) — run the long-lived sebas service.
/// Flags live here for backward compatibility with the pre-subcommand CLI.
#[derive(Parser)]
struct RunArgs {
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

#[derive(Subcommand)]
enum Cmd {
    /// Install a systemd user or system unit for sebas.
    InstallService(InstallServiceArgs),
}

#[derive(Parser)]
struct InstallServiceArgs {
    /// Install as a user unit (~/.config/systemd/user/sebas.service).
    #[arg(long, conflicts_with = "system")]
    user: bool,

    /// Install as a system unit (/etc/systemd/system/sebas.service). Requires root.
    #[arg(long, conflicts_with = "user")]
    system: bool,

    /// After installing, also `systemctl enable` and `start` the unit.
    #[arg(long)]
    auto_start: bool,

    /// Run the system unit as this user/group (system scope only).
    #[arg(long)]
    run_as: Option<String>,

    /// Overwrite an existing unit file.
    #[arg(long)]
    force: bool,

    /// Path to the sebas config.toml to bake into ExecStart. Must be absolute.
    #[arg(long, default_value = "./config.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Some(Cmd::InstallService(args)) => {
            // Map install-service's own exit codes (2/3/4/5/6) up to the
            // process. Defaults to 1 for any other failure.
            let err = match sebas::install_service::run(args.into()).await {
                Ok(()) => return Ok(()),
                Err(e) => e,
            };
            let code = sebas::install_service::exit_code_of(&err).unwrap_or(1);
            eprintln!("error: {err:?}");
            std::process::exit(code);
        }
        None => {
            // Default mode: long-lived run.
            let run = RunArgs::parse();
            let raw = std::fs::read_to_string(&run.config).unwrap_or_else(|_| {
                // No file -> use minimal defaults; require env vars for credentials
                let app_id = std::env::var("SEBAS_FEISHU_APP_ID").unwrap_or_default();
                let app_secret = std::env::var("SEBAS_FEISHU_APP_SECRET").unwrap_or_default();
                format!(
                    "[feishu]\napp_id = \"{app_id}\"\napp_secret = \"{app_secret}\"\nowner_id = \"ou_xxx\"\n"
                )
            });
            let cfg = sebas::config::Config::parse(&raw).map_err(|e| anyhow::anyhow!("{e}"))?;
            sebas::run::run(cfg, run.test_msg, run.dump_inbound)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(())
        }
    }
}

impl From<InstallServiceArgs> for sebas::install_service::InstallServiceArgs {
    fn from(a: InstallServiceArgs) -> Self {
        Self {
            user: a.user,
            system: a.system,
            auto_start: a.auto_start,
            force: a.force,
            run_as: a.run_as,
            config: a.config,
        }
    }
}
