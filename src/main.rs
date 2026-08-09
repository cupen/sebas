mod cli;

use clap::Parser;
use cli::{Cli, Cmd, GatewayArgs, InstallServiceArgs, RecordArgs, ReplayArgs};
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
            sebas::run::run(cfg, raw, run.test_msg, run.dump_inbound, gateway_cfg)
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

impl From<GatewayArgs> for sebas::gateway_cmd::GatewayArgs {
    fn from(a: GatewayArgs) -> Self {
        Self {
            config: a.config,
            debug: a.debug,
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
}
