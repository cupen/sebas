mod cli;

use clap::Parser;
use cli::{
    Cli, Cmd, ControlArgs, ControlCmd, ControlStatusArgs, GatewayArgs, OutputFormat, RecordArgs,
    ReplayArgs, ServiceArgs, WebUiArgs,
};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Service(args) => {
            // Map `service`'s own exit codes (2/3/4/5/6) up to the process.
            // Defaults to 1 for any other failure.
            let args = sebas::service::Args::from(args);
            let err = if args.install {
                match sebas::service::run_install(args).await {
                    Ok(()) => return Ok(()),
                    Err(e) => e,
                }
            } else {
                match sebas::service::run_uninstall(args).await {
                    Ok(()) => return Ok(()),
                    Err(e) => e,
                }
            };
            let code = sebas::service::exit_code_of(&err).unwrap_or(1);
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
        Cmd::WebUi(args) => {
            if let Err(e) = sebas::webui_cmd::run(args.into()).await {
                eprintln!("error: {e:?}");
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Control(args) => run_control(args).await,
        Cmd::Status(args) => run_control_status(args, ControlCmd::Status).await,
        Cmd::Services(args) => run_control_status(args, ControlCmd::Services).await,
        Cmd::Ctl(args) => run_control(args).await,
        Cmd::Watchdog(args) => {
            let raw = std::fs::read_to_string(&args.config).unwrap_or_default();
            let cfg = sebas::config::Config::parse(&raw).map_err(|e| anyhow::anyhow!("{e}"))?;
            sebas::watchdog::run_watchdog(cfg.watchdog, args.config, args.debug)
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
                    sebas_gateway::config::GatewayConfig::parse(&raw)
                        .map_err(|e| anyhow::anyhow!("{e}"))?,
                )
            } else {
                None
            };
            if run.debug
                && let Some(c) = gateway_cfg.as_mut()
            {
                sebas_gateway::debug::enable_debug_test_provider(c);
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

    // Resolve socket: --socket > $SEBAS_CONTROL_SOCKET > XDG default.
    let path = args
        .socket
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("SEBAS_CONTROL_SOCKET")
                .ok()
                .map(PathBuf::from)
        })
        .unwrap_or_else(default_socket_path);

    // Resolve secret: --secret > $SEBAS_CONTROL_SECRET.
    // Watchdog intentionally does not persist this (see openspec/specs/watchdog/spec.md), so a CLI
    // client without the env var must be told to export it explicitly.
    let secret = match args
        .secret
        .or_else(|| std::env::var("SEBAS_CONTROL_SECRET").ok())
    {
        Some(s) if !s.is_empty() => s,
        _ => {
            return Err(friendly_error(
                "missing control RPC secret",
                "pass --secret or set SEBAS_CONTROL_SECRET (watchdog prints this env to its own child via stderr; for an external CLI use the value the watchdog started with)",
            ));
        }
    };

    let uid = current_uid();
    let envelope = match args.cmd {
        ControlCmd::Status => ControlEnvelope {
            version: 1,
            request_id: "cli_status".into(),
            secret: secret.clone(),
            actor: RpcActor::Cli { uid },
            request: RpcControlRequest::Status,
        },
        ControlCmd::Events { since } => ControlEnvelope {
            version: 1,
            request_id: "cli_events".into(),
            secret: secret.clone(),
            actor: RpcActor::Cli { uid },
            request: RpcControlRequest::EventsSince { seq: since },
        },
        ControlCmd::Update { dev, dry_run } => ControlEnvelope {
            version: 1,
            request_id: "cli_update".into(),
            secret: secret.clone(),
            actor: RpcActor::Cli { uid },
            request: RpcControlRequest::Update { dev, dry_run },
        },
        ControlCmd::Rollback { dry_run } => ControlEnvelope {
            version: 1,
            request_id: "cli_rollback".into(),
            secret: secret.clone(),
            actor: RpcActor::Cli { uid },
            request: RpcControlRequest::Rollback { dry_run },
        },
        ControlCmd::RestartCore => ControlEnvelope {
            version: 1,
            request_id: "cli_restart_core".into(),
            secret: secret.clone(),
            actor: RpcActor::Cli { uid },
            request: RpcControlRequest::RestartCore,
        },
        ControlCmd::Services => ControlEnvelope {
            version: 1,
            request_id: "cli_services".into(),
            secret: secret.clone(),
            actor: RpcActor::Cli { uid },
            request: RpcControlRequest::ServiceStatus,
        },
    };

    let response = match request(&path, &envelope).await {
        Ok(r) => r,
        Err(e) => return Err(friendly_rpc_error(e.into(), &path)),
    };

    render_response(args.format, &response);

    // Map watchdog-side rejections to exit code 2 so scripts can detect them
    // without parsing strings.
    if matches!(response, RpcControlResponse::Rejected { .. }) {
        std::process::exit(2);
    }
    Ok(())
}

/// 顶层 `sebas status` / `sebas services`：构造一个固定的 ControlArgs
/// 复用到 `run_control`，避免复制稳定信封构造逻辑。
async fn run_control_status(args: ControlStatusArgs, cmd: ControlCmd) -> anyhow::Result<()> {
    let control = ControlArgs {
        socket: args.socket,
        secret: args.secret,
        format: args.format,
        cmd,
    };
    run_control(control).await
}

fn render_response(
    format: OutputFormat,
    response: &sebas::watchdog::control_rpc::RpcControlResponse,
) {
    use sebas::watchdog::control_rpc::RpcControlResponse;
    match format {
        OutputFormat::Json => {
            // Raw envelope; stable schema, machine-readable. serde_json's
            // default formatter is preserved so downstream tooling can rely on
            // field order / casing.
            let json = serde_json::to_string_pretty(response)
                .expect("RpcControlResponse is always serializable");
            println!("{json}");
        }
        OutputFormat::Human => match response {
            RpcControlResponse::Accepted {
                operation_id,
                status,
            } => {
                println!("accepted operation={operation_id} status={status}");
            }
            RpcControlResponse::Rejected { code, message } => {
                eprintln!("rejected code={code} message={message}");
            }
            RpcControlResponse::Events { events } => {
                if events.is_empty() {
                    println!("(no events)");
                    return;
                }
                for event in events {
                    println!(
                        "#{} [{}] {} {}",
                        event.seq, event.kind, event.operation_id, event.public_message
                    );
                }
            }
            RpcControlResponse::Services { services } => {
                if services.is_empty() {
                    println!("(no managed services)");
                    return;
                }
                for svc in services {
                    println!(
                        "- {}: {} (desired: {}){}",
                        svc.name,
                        svc.status,
                        svc.desired,
                        svc.uptime_secs
                            .map(|u| format!(" uptime={u}s"))
                            .unwrap_or_default()
                    );
                }
            }
            RpcControlResponse::PendingConfirmation {
                action,
                message,
                expires_in,
                ..
            } => {
                println!("confirmation required action={action} expires_in={expires_in}s");
                println!("{message}");
            }
        },
    }
}

/// Convert an anyhow-wrapped `SebasError` into a human-readable message plus
/// the next-best action. CLI users hit three recurring failure modes:
/// missing secret, missing/unwritable socket, and connection-refused.
fn friendly_rpc_error(err: anyhow::Error, socket: &std::path::Path) -> anyhow::Error {
    use sebas::error::SebasError;
    use std::io::ErrorKind;

    if let Some(se) = err.downcast_ref::<SebasError>() {
        match se {
            SebasError::Io(io_err) => match io_err.kind() {
                ErrorKind::NotFound => {
                    return anyhow::anyhow!(
                        "watchdog control socket not found at {}\n\
                         hint: is `sebas watchdog` running? override path with --socket \
                         or $SEBAS_CONTROL_SOCKET",
                        socket.display()
                    );
                }
                ErrorKind::ConnectionRefused => {
                    return anyhow::anyhow!(
                        "watchdog control socket refused connection at {}\n\
                         hint: the socket exists but watchdog is not listening; \
                         check `sebas watchdog` logs",
                        socket.display()
                    );
                }
                ErrorKind::PermissionDenied => {
                    return anyhow::anyhow!(
                        "permission denied connecting to watchdog socket at {}\n\
                         hint: socket is mode 0600 owned by the user running \
                         `sebas watchdog`; run as that user, via sudo -u, or \
                         systemd RunAsUser",
                        socket.display()
                    );
                }
                _ => {}
            },
            SebasError::Upgrade(msg) if msg.contains("closed without response") => {
                return anyhow::anyhow!(
                    "watchdog closed the socket without sending a response\n\
                     hint: the daemon may be shutting down or the per-instance \
                     secret rotated after restart (watchdog secrets are not \
                     persisted by design)"
                );
            }
            SebasError::Upgrade(msg) if msg.contains("parse response failed") => {
                return anyhow::anyhow!(
                    "watchdog returned a non-JSON response: {}\n\
                     hint: watchdog and CLI versions may be out of sync; \
                     rebuild from the same commit",
                    msg
                );
            }
            _ => {}
        }
    }

    anyhow::anyhow!(
        "control RPC request to {} failed: {}",
        socket.display(),
        err
    )
}

fn friendly_error(what: &str, hint: &str) -> anyhow::Error {
    anyhow::anyhow!("{what}\nhint: {hint}")
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

impl From<ServiceArgs> for sebas::service::Args {
    fn from(a: ServiceArgs) -> Self {
        Self {
            install: a.install,
            uninstall: a.uninstall,
            user: a.user,
            auto_start: a.auto_start,
            force: a.force,
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

impl From<WebUiArgs> for sebas::webui_cmd::WebUiArgs {
    fn from(a: WebUiArgs) -> Self {
        Self::new(a.config)
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

    /// The watchdog spawns its child and `service` bakes an `ExecStart`
    /// both keyed on `sebas::CORE_SUBCOMMAND`; the clap subcommand here must
    /// agree or the supervisor and unit silently target a different argv. If
    /// this fails, rename `Cmd::Run` and `CORE_SUBCOMMAND` together.
    #[test]
    fn run_subcommand_name_matches_core_subcommand_const() {
        let cli = Cli::try_parse_from(["sebas", sebas::CORE_SUBCOMMAND, "--config", "x.toml"])
            .expect("`sebas {CORE_SUBCOMMAND} --config <path>` must parse");
        assert!(matches!(cli.cmd, Cmd::Run(_)));
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

    #[test]
    fn webui_subcommand_accepts_config_flag() {
        let cli = Cli::try_parse_from(["sebas", "webui", "--config", "x.toml"])
            .expect("`sebas webui --config <path>` must parse");
        let Cmd::WebUi(args) = cli.cmd else {
            panic!("expected WebUi subcommand");
        };
        assert_eq!(args.config, "x.toml");
    }

    #[test]
    fn webui_subcommand_short_config_flag() {
        let cli = Cli::try_parse_from(["sebas", "webui", "-c", "x.toml"])
            .expect("`sebas webui -c <path>` must parse");
        let Cmd::WebUi(args) = cli.cmd else {
            panic!("expected WebUi subcommand");
        };
        assert_eq!(args.config, "x.toml");
    }

    #[test]
    fn webui_subcommand_defaults_to_cwd_config_toml() {
        let cli = Cli::try_parse_from(["sebas", "webui"]).expect("bare `sebas webui` must parse");
        let Cmd::WebUi(args) = cli.cmd else {
            panic!("expected WebUi subcommand");
        };
        assert_eq!(args.config, "./config.toml");
    }

    #[test]
    fn run_no_webui_flag_parses() {
        let cli = Cli::try_parse_from(["sebas", "run", "--no-webui", "-c", "x.toml"])
            .expect("`sebas run --no-webui` must parse");
        let Cmd::Run(args) = cli.cmd else {
            panic!("expected Run subcommand");
        };
        assert!(args.no_webui, "--no-webui 应被解析");
        assert!(!args.webui, "默认 webui 应为 false");
    }

    #[test]
    fn run_webui_and_no_webui_conflict() {
        assert!(
            Cli::try_parse_from(["sebas", "run", "--webui", "--no-webui", "-c", "x.toml"]).is_err(),
            "--webui 与 --no-webui 互斥"
        );
    }

    // -----------------------------------------------------------------
    // sebas-npc: public CLI client (Phase 6 Task 6.1)
    // -----------------------------------------------------------------

    #[test]
    fn control_restart_core_subcommand_parses() {
        let cli = Cli::try_parse_from(["sebas", "control", "restart-core"])
            .expect("`sebas control restart-core` must parse");
        let Cmd::Control(args) = cli.cmd else {
            panic!("expected Control subcommand");
        };
        assert!(matches!(args.cmd, ControlCmd::RestartCore));
    }

    #[test]
    fn control_services_subcommand_parses() {
        let cli = Cli::try_parse_from(["sebas", "control", "services"])
            .expect("`sebas control services` must parse");
        let Cmd::Control(args) = cli.cmd else {
            panic!("expected Control subcommand");
        };
        assert!(matches!(args.cmd, ControlCmd::Services));
    }

    #[test]
    fn top_level_status_parses() {
        let cli = Cli::try_parse_from(["sebas", "status"]).expect("`sebas status` must parse");
        let Cmd::Status(args) = cli.cmd else {
            panic!("expected Status subcommand");
        };
        assert_eq!(args.format, OutputFormat::Human);
    }

    #[test]
    fn top_level_status_accepts_format_flag() {
        let cli = Cli::try_parse_from(["sebas", "status", "--format", "json"]).expect("must parse");
        let Cmd::Status(args) = cli.cmd else {
            panic!("expected Status subcommand");
        };
        assert_eq!(args.format, OutputFormat::Json);
    }

    #[test]
    fn top_level_status_accepts_socket_and_secret_flags() {
        let cli = Cli::try_parse_from([
            "sebas",
            "status",
            "--socket",
            "/tmp/x.sock",
            "--secret",
            "s",
        ])
        .expect("must parse");
        let Cmd::Status(args) = cli.cmd else {
            panic!("expected Status subcommand");
        };
        assert_eq!(args.socket.as_deref(), Some("/tmp/x.sock"));
        assert_eq!(args.secret.as_deref(), Some("s"));
    }

    #[test]
    fn top_level_services_parses() {
        let cli = Cli::try_parse_from(["sebas", "services"]).expect("`sebas services` must parse");
        let Cmd::Services(_) = cli.cmd else {
            panic!("expected Services subcommand");
        };
    }

    #[test]
    fn top_level_ctl_aliases_control() {
        let cli =
            Cli::try_parse_from(["sebas", "ctl", "status"]).expect("`sebas ctl status` must parse");
        let Cmd::Ctl(args) = cli.cmd else {
            panic!("expected Ctl subcommand");
        };
        assert!(matches!(args.cmd, ControlCmd::Status));
    }

    #[test]
    fn control_format_defaults_to_human() {
        let cli =
            Cli::try_parse_from(["sebas", "control", "status"]).expect("parse with no --format");
        let Cmd::Control(args) = cli.cmd else {
            panic!("expected Control");
        };
        assert_eq!(args.format, OutputFormat::Human);
    }

    #[test]
    fn control_format_json_parses() {
        let cli =
            Cli::try_parse_from(["sebas", "control", "status", "--format", "json"]).expect("json");
        let Cmd::Control(args) = cli.cmd else {
            panic!("expected Control");
        };
        assert_eq!(args.format, OutputFormat::Json);
    }

    #[test]
    fn control_format_human_explicit() {
        let cli = Cli::try_parse_from(["sebas", "control", "services", "--format", "human"])
            .expect("human");
        let Cmd::Control(args) = cli.cmd else {
            panic!("expected Control");
        };
        assert_eq!(args.format, OutputFormat::Human);
    }

    #[test]
    fn control_format_invalid_value_rejected() {
        assert!(Cli::try_parse_from(["sebas", "control", "status", "--format", "yaml"]).is_err());
    }

    #[test]
    fn control_unknown_subcommand_rejected() {
        // Snapshot: only the documented Phase 6 subcommands exist.
        assert!(Cli::try_parse_from(["sebas", "control", "purge-everything"]).is_err());
    }

    #[test]
    fn control_socket_and_secret_flags_are_optional() {
        // Required secret/socket resolution happens at runtime, not parse time.
        let cli = Cli::try_parse_from(["sebas", "control", "status"]).expect("parse");
        let Cmd::Control(args) = cli.cmd else {
            panic!("expected Control");
        };
        assert!(args.socket.is_none());
        assert!(args.secret.is_none());
    }

    #[test]
    fn control_socket_and_secret_flags_capture_values() {
        let cli = Cli::try_parse_from([
            "sebas",
            "control",
            "--socket",
            "/tmp/test.sock",
            "--secret",
            "abc",
            "status",
        ])
        .expect("parse");
        let Cmd::Control(args) = cli.cmd else {
            panic!("expected Control");
        };
        assert_eq!(args.socket.as_deref(), Some("/tmp/test.sock"));
        assert_eq!(args.secret.as_deref(), Some("abc"));
    }

    #[test]
    fn rpc_control_response_json_schema_stable() {
        // The JSON envelope is the contract Phase 6 promises downstream tools
        // can rely on. Pin field names + types for every variant.
        use sebas::watchdog::control_rpc::{RpcControlEvent, RpcControlResponse, RpcServiceStatus};

        let cases: Vec<(&'static str, RpcControlResponse)> = vec![
            (
                "accepted",
                RpcControlResponse::Accepted {
                    operation_id: "op_42".into(),
                    status: "Running".into(),
                },
            ),
            (
                "rejected",
                RpcControlResponse::Rejected {
                    code: "unauthorized".into(),
                    message: "missing or invalid control RPC secret".into(),
                },
            ),
            (
                "events",
                RpcControlResponse::Events {
                    events: vec![RpcControlEvent {
                        seq: 7,
                        operation_id: "op_42".into(),
                        kind: "Started".into(),
                        public_message: "updater started".into(),
                    }],
                },
            ),
            (
                "services",
                RpcControlResponse::Services {
                    services: vec![RpcServiceStatus {
                        name: "gateway".into(),
                        status: "Running".into(),
                        desired: "Enabled".into(),
                        uptime_secs: Some(120),
                    }],
                },
            ),
        ];

        // Round-trip each variant: every field that survives a parse is part
        // of the contract. If a field is added or renamed, this test breaks.
        for (label, original) in &cases {
            let json = serde_json::to_string(original).expect("serialize");
            let parsed: RpcControlResponse = serde_json::from_str(&json).expect(label);
            assert_eq!(&parsed, original, "round-trip mismatch for {label}");
        }

        // Spot-check field casing/structure for `accepted` since that's the
        // most machine-consumed variant.
        let json = serde_json::to_string(&RpcControlResponse::Accepted {
            operation_id: "op_x".into(),
            status: "Running".into(),
        })
        .unwrap();
        assert!(
            json.contains("\"type\":\"accepted\""),
            "tag field path: {json}"
        );
        assert!(json.contains("\"operation_id\":\"op_x\""));
        assert!(json.contains("\"status\":\"Running\""));
    }

    #[test]
    fn friendly_rpc_error_socket_not_found_mentions_running_watchdog() {
        use std::path::Path;
        let err = friendly_rpc_error(
            anyhow::anyhow!(sebas::error::SebasError::Io(std::io::Error::from(
                std::io::ErrorKind::NotFound
            ))),
            Path::new("/var/run/sebas/missing.sock"),
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("watchdog control socket not found"),
            "expected not-found message, got: {msg}"
        );
        assert!(
            msg.contains("is `sebas watchdog` running"),
            "hint missing: {msg}"
        );
    }

    #[test]
    fn friendly_rpc_error_connection_refused_mentions_watchdog_logs() {
        use std::path::Path;
        let err = friendly_rpc_error(
            anyhow::anyhow!(sebas::error::SebasError::Io(std::io::Error::from(
                std::io::ErrorKind::ConnectionRefused
            ))),
            Path::new("/tmp/sebas.sock"),
        );
        let msg = format!("{err}");
        assert!(msg.contains("refused connection"), "got: {msg}");
        assert!(msg.contains("watchdog"), "got: {msg}");
    }

    #[test]
    fn friendly_rpc_error_permission_denied_mentions_user_match() {
        use std::path::Path;
        let err = friendly_rpc_error(
            anyhow::anyhow!(sebas::error::SebasError::Io(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied
            ))),
            Path::new("/tmp/sebas.sock"),
        );
        let msg = format!("{err}");
        assert!(msg.contains("permission denied"), "got: {msg}");
        assert!(msg.contains("sudo -u"), "hint missing: {msg}");
    }

    #[test]
    fn friendly_error_includes_hint() {
        let err = friendly_error("missing control RPC secret", "set SEBAS_CONTROL_SECRET");
        let msg = format!("{err}");
        assert!(msg.contains("missing control RPC secret"));
        assert!(msg.contains("hint: set SEBAS_CONTROL_SECRET"));
    }
}
