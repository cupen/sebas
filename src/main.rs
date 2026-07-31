use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    /// Replay captured inbound events from a directory of `.json` files.
    Replay(ReplayArgs),
    /// Record an ACP agent's stdio traffic as a fixture file (spec §4.4).
    /// Type/paste JSON-RPC lines on stdin; responses print to stdout; both
    /// directions are appended to --output as {"dir","msg"} journal lines.
    Record(RecordArgs),
}

#[derive(Parser)]
struct RecordArgs {
    /// Fixture file to write (JSONL, one {"dir","msg"} object per line).
    #[arg(long)]
    output: String,

    /// Config supplying acp.claude.path/args for the agent to record.
    #[arg(long, default_value = "./config.toml")]
    config: String,

    /// Extra args for the agent binary, after `--`
    /// (appended to the configured acp.claude.args).
    #[arg(last = true)]
    agent_args: Vec<String>,
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

#[derive(Parser)]
struct ReplayArgs {
    /// Directory containing `.json` files to replay (one envelope per file).
    /// Files are processed in lexical filename order so timestamp-prefixed
    /// dumps preserve capture order.
    #[arg(long)]
    dir: String,
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
        Some(Cmd::Replay(args)) => {
            // Exit 1 only on dir-not-found / unwrap-able FS errors.
            // Per-frame read/parse failures are warn-and-skip inside run().
            if let Err(e) = sebas::replay::run(args.into()).await {
                eprintln!("error: {e:?}");
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Cmd::Record(args)) => {
            if let Err(e) = sebas::record::run(args.into()).await {
                eprintln!("error: {e:?}");
                std::process::exit(1);
            }
            Ok(())
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

impl From<ReplayArgs> for sebas::replay::ReplayArgs {
    fn from(a: ReplayArgs) -> Self {
        Self {
            dir: PathBuf::from(a.dir),
        }
    }
}

impl From<RecordArgs> for sebas::record::RecordArgs {
    fn from(a: RecordArgs) -> Self {
        Self {
            output: PathBuf::from(a.output),
            config: a.config,
            agent_args: a.agent_args,
        }
    }
}
