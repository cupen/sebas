mod cli;

use clap::Parser;
use cli::{
    Cli, Cmd, ControlArgs, ControlCmd, GatewayArgs, InstallServiceArgs, RecordArgs, ReplayArgs,
};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::InstallService(args) => {
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
        Cmd::Replay(args) => {
            // Exit 1 only on dir-not-found / unwrap-able FS errors.
            // Per-frame read/parse failures are warn-and-skip inside run().
            if let Err(e) = sebas::replay::run(args.into()).await {
                eprintln!("error: {e:?}");
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Record(args) => {
            if let Err(e) = sebas::record::run(args.into()).await {
                eprintln!("error: {e:?}");
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Gateway(args) => {
            if let Err(e) = sebas::gateway_cmd::run(args.into()).await {
                eprintln!("error: {e:?}");
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Control(args) => run_control(args).await,
        Cmd::Watchdog(args) => {
            let raw = std::fs::read_to_string(&args.config).unwrap_or_default();
            let cfg = sebas::config::Config::parse(&raw).map_err(|e| anyhow::anyhow!("{e}"))?;
            sebas::watchdog::run_watchdog(cfg.watchdog, args.config)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(())
        }
        Cmd::Update(args) => {
            sebas::update::run(args.into())
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(())
        }
        Cmd::Run(run) => {
            let raw = std::fs::read_to_string(&run.config).unwrap_or_else(|_| {
                // No file -> use minimal defaults; require env vars for credentials
                let app_id = std::env::var("SEBAS_FEISHU_APP_ID").unwrap_or_default();
                let app_secret = std::env::var("SEBAS_FEISHU_APP_SECRET").unwrap_or_default();
                format!(
                    "[feishu]\napp_id = \"{app_id}\"\napp_secret = \"{app_secret}\"\nowner_id = \"ou_xxx\"\n"
                )
            });
            let cfg = sebas::config::Config::parse(&raw).map_err(|e| anyhow::anyhow!("{e}"))?;
            let mut gateway_cfg = if run.gateway {
                Some(
                    gateway::config::GatewayConfig::parse(&raw)
                        .map_err(|e| anyhow::anyhow!("{e}"))?,
                )
            } else {
                None
            };
            if run.debug
                && let Some(c) = gateway_cfg.as_mut()
            {
                c.enable_debug_test_provider();
            }
            sebas::run::run(
                cfg,
                raw,
                run.test_msg,
                run.dump_inbound,
                gateway_cfg,
                run.webui,
                run.webui_port,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(())
        }
    }
}

async fn run_control(args: ControlArgs) -> anyhow::Result<()> {
    use sebas::watchdog::control_rpc::{
        ControlEnvelope, RpcActor, RpcControlRequest, RpcControlResponse, default_socket_path,
        request,
    };

    let path = args
        .socket
        .map(PathBuf::from)
        .unwrap_or_else(default_socket_path);
    let uid = current_uid();
    let envelope = match args.cmd {
        ControlCmd::Status => ControlEnvelope {
            version: 1,
            request_id: "cli_status".into(),
            actor: RpcActor::Cli { uid },
            request: RpcControlRequest::Status,
        },
        ControlCmd::Events { since } => ControlEnvelope {
            version: 1,
            request_id: "cli_events".into(),
            actor: RpcActor::Cli { uid },
            request: RpcControlRequest::EventsSince { seq: since },
        },
        ControlCmd::Update { dev, dry_run } => ControlEnvelope {
            version: 1,
            request_id: "cli_update".into(),
            actor: RpcActor::Cli { uid },
            request: RpcControlRequest::Update { dev, dry_run },
        },
        ControlCmd::Rollback { dry_run } => ControlEnvelope {
            version: 1,
            request_id: "cli_rollback".into(),
            actor: RpcActor::Cli { uid },
            request: RpcControlRequest::Rollback { dry_run },
        },
    };

    match request(&path, &envelope).await? {
        RpcControlResponse::Accepted {
            operation_id,
            status,
        } => {
            println!("accepted operation={operation_id} status={status}");
        }
        RpcControlResponse::Rejected { code, message } => {
            eprintln!("rejected code={code} message={message}");
            std::process::exit(2);
        }
        RpcControlResponse::Events { events } => {
            for event in events {
                println!(
                    "#{} [{}] {} {}",
                    event.seq, event.kind, event.operation_id, event.public_message
                );
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
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

impl From<GatewayArgs> for sebas::gateway_cmd::GatewayArgs {
    fn from(a: GatewayArgs) -> Self {
        Self {
            config: a.config,
            debug: a.debug,
        }
    }
}

impl From<cli::UpdateArgs> for sebas::update::UpdateArgs {
    fn from(a: cli::UpdateArgs) -> Self {
        Self {
            config: a.config,
            dev: a.dev,
            dry_run: a.dry_run,
            rollback: a.rollback,
            project_dir: a.project_dir.map(PathBuf::from),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_subcommand_accepts_config_flag() {
        let cli = Cli::try_parse_from(["sebas", "run", "--config", "x.toml"])
            .expect("`sebas run --config <path>` must parse");
        let Cmd::Run(args) = cli.cmd else {
            panic!("expected Run subcommand");
        };
        assert_eq!(args.config, "x.toml");
    }

    #[test]
    fn run_subcommand_config_defaults_to_cwd_config_toml() {
        let cli = Cli::try_parse_from(["sebas", "run"]).expect("bare `run` must parse");
        let Cmd::Run(args) = cli.cmd else {
            panic!("expected Run subcommand");
        };
        assert_eq!(args.config, "./config.toml");
    }

    #[test]
    fn run_subcommand_accepts_gateway_flag() {
        let cli = Cli::try_parse_from(["sebas", "run", "--gateway", "--config", "x.toml"])
            .expect("`sebas run --gateway --config <path>` must parse");
        let Cmd::Run(args) = cli.cmd else {
            panic!("expected Run subcommand");
        };
        assert!(args.gateway, "--gateway flag must be captured");
    }

    #[test]
    fn run_subcommand_accepts_short_config_flag() {
        let cli = Cli::try_parse_from(["sebas", "run", "-c", "x.toml"])
            .expect("`sebas run -c <path>` must parse");
        let Cmd::Run(args) = cli.cmd else {
            panic!("expected Run subcommand");
        };
        assert_eq!(args.config, "x.toml");
    }

    #[test]
    fn top_level_config_flag_is_rejected() {
        // 入口是显式 run 子命令；顶层不再吞 run 模式的 flags。
        assert!(Cli::try_parse_from(["sebas", "--config", "x.toml"]).is_err());
    }

    #[test]
    fn gateway_subcommand_accepts_config_flag() {
        let cli = Cli::try_parse_from(["sebas", "gateway", "--config", "x.toml"])
            .expect("`sebas gateway --config <path>` must parse");
        let Cmd::Gateway(args) = cli.cmd else {
            panic!("expected Gateway subcommand");
        };
        assert_eq!(args.config, "x.toml");
    }

    #[test]
    fn gateway_subcommand_accepts_short_config_flag() {
        let cli = Cli::try_parse_from(["sebas", "gateway", "-c", "x.toml"])
            .expect("`sebas gateway -c <path>` must parse");
        let Cmd::Gateway(args) = cli.cmd else {
            panic!("expected Gateway subcommand");
        };
        assert_eq!(args.config, "x.toml");
    }

    #[test]
    fn gateway_subcommand_accepts_debug_flag() {
        let cli = Cli::try_parse_from(["sebas", "gateway", "--debug", "-c", "x.toml"])
            .expect("`sebas gateway --debug -c <path>` must parse");
        let Cmd::Gateway(args) = cli.cmd else {
            panic!("expected Gateway subcommand");
        };
        assert!(args.debug, "--debug flag must be captured");
    }

    #[test]
    fn control_status_subcommand_parses() {
        let cli = Cli::try_parse_from(["sebas", "control", "status"])
            .expect("`sebas control status` must parse");
        let Cmd::Control(args) = cli.cmd else {
            panic!("expected Control subcommand");
        };
        assert!(matches!(args.cmd, ControlCmd::Status));
    }

    #[test]
    fn control_update_dev_subcommand_parses() {
        let cli = Cli::try_parse_from(["sebas", "control", "update", "--dev", "--dry-run"])
            .expect("`sebas control update --dev --dry-run` must parse");
        let Cmd::Control(args) = cli.cmd else {
            panic!("expected Control subcommand");
        };
        assert!(matches!(
            args.cmd,
            ControlCmd::Update {
                dev: true,
                dry_run: true
            }
        ));
    }

    #[test]
    fn update_subcommand_accepts_dev_project_dir_and_dry_run() {
        let cli = Cli::try_parse_from([
            "sebas",
            "update",
            "--dev",
            "--dry-run",
            "--project-dir",
            "/tmp/sebas",
            "--config",
            "x.toml",
        ])
        .expect("`sebas update --dev --dry-run --project-dir <path>` must parse");
        let Cmd::Update(args) = cli.cmd else {
            panic!("expected Update subcommand");
        };
        assert!(args.dev);
        assert!(args.dry_run);
        assert_eq!(args.project_dir.as_deref(), Some("/tmp/sebas"));
        assert_eq!(args.config, "x.toml");
    }

    #[test]
    fn update_rollback_conflicts_with_dev() {
        assert!(Cli::try_parse_from(["sebas", "update", "--rollback", "--dev"]).is_err());
    }
}
