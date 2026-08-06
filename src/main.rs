mod cli;

use clap::Parser;
use cli::{Cli, Cmd, InstallServiceArgs, RecordArgs, ReplayArgs};
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
    fn top_level_config_flag_is_rejected() {
        // 入口是显式 run 子命令；顶层不再吞 run 模式的 flags。
        assert!(Cli::try_parse_from(["sebas", "--config", "x.toml"]).is_err());
    }
}
