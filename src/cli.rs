use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Run the long-lived sebas service.
    Run(RunArgs),
    /// Install a systemd user or system unit for sebas.
    InstallService(InstallServiceArgs),
    /// Replay captured inbound events from a directory of `.json` files.
    Replay(ReplayArgs),
    /// Record an ACP agent's stdio traffic as a fixture file (spec §4.4).
    /// Type/paste JSON-RPC lines on stdin; responses print to stdout; both
    /// directions are appended to --output as {"dir","msg"} journal lines.
    Record(RecordArgs),
    /// Run the LLM provider gateway (Anthropic/OpenAI dual-protocol
    /// transparent proxy). See docs/superpowers/specs/2026-08-07-gateway-design.md.
    Gateway(GatewayArgs),
    /// Start the watchdog daemon.
    Watchdog(WatchdogArgs),
    /// One-shot update implementation used by watchdog.
    Update(UpdateArgs),
    /// Send a command to the watchdog control plane.
    Control(ControlArgs),
}

/// Run mode — the long-lived sebas service.
#[derive(Parser)]
pub struct RunArgs {
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,

    /// 同时在随机端口（127.0.0.1:0）上启动内置 gateway；实际端口在日志中输出。
    /// provider 从配置顶层 `[provider.*]` 读取。
    #[arg(long)]
    pub gateway: bool,

    /// 同时让内置 gateway 进入 debug 模式：增加 `test` 模型，由 gateway 自身
    /// 应答（固定文字 + 回显输入），不转发外部上游。
    #[arg(long)]
    pub debug: bool,

    /// Send a startup "sebas 已启动" message to this chat_id, then continue running.
    /// Useful for verifying outbound is wired correctly. chat_id format depends on
    /// receive_id_type: open_id (private) or chat_id (group).
    #[arg(long)]
    pub test_msg: Option<String>,

    /// Start the WebUI dashboard server.
    #[arg(long)]
    pub webui: bool,

    /// Port for the WebUI server (default: 9797).
    #[arg(long, default_value = "9797")]
    pub webui_port: u16,

    /// Dump every raw inbound WS payload to this directory as one .json file per
    /// event (timestamp-prefixed). Useful for local replay/debug without needing
    /// the live Feishu connection. Disabled when omitted.
    #[arg(long)]
    pub dump_inbound: Option<String>,
}

#[derive(Parser)]
pub struct InstallServiceArgs {
    /// Install as a user unit (~/.config/systemd/user/sebas.service).
    #[arg(long, conflicts_with = "system")]
    pub user: bool,

    /// Install as a system unit (/etc/systemd/system/sebas.service). Requires root.
    #[arg(long, conflicts_with = "user")]
    pub system: bool,

    /// After installing, also `systemctl enable` and `start` the unit.
    #[arg(long)]
    pub auto_start: bool,

    /// Run the system unit as this user/group (system scope only).
    #[arg(long)]
    pub run_as: Option<String>,

    /// Overwrite an existing unit file.
    #[arg(long)]
    pub force: bool,

    /// Path to the sebas config.toml to bake into ExecStart. Must be absolute.
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,
}

#[derive(Parser)]
pub struct ReplayArgs {
    /// Directory containing `.json` files to replay (one envelope per file).
    /// Files are processed in lexical filename order so timestamp-prefixed
    /// dumps preserve capture order.
    #[arg(long)]
    pub dir: String,
}

#[derive(Parser)]
pub struct RecordArgs {
    /// Fixture file to write (JSONL, one {"dir","msg"} object per line).
    #[arg(long)]
    pub output: String,

    /// Config supplying acp.claude.path/args for the agent to record.
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,

    /// Extra args for the agent binary, after `--`
    /// (appended to the configured acp.claude.args).
    #[arg(last = true)]
    pub agent_args: Vec<String>,
}

/// `sebas gateway` — run the LLM provider gateway.
#[derive(Parser)]
pub struct GatewayArgs {
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,

    /// 启用 debug 模式：增加内置 `test` 模型，gateway 自身应答
    /// （固定文字 + 回显输入），不转发外部上游。
    #[arg(long)]
    pub debug: bool,
}

/// `sebas watchdog` — start the watchdog daemon.
/// Manages the sebas child process and handles self-upgrade.
#[derive(Parser)]
pub struct WatchdogArgs {
    /// Path to the sebas config.toml.
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,
}

/// `sebas update` — one-shot update implementation used by watchdog.
#[derive(Parser)]
pub struct UpdateArgs {
    /// Path to the sebas config.toml.
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,

    /// Build from a local checkout instead of downloading a release.
    #[arg(long)]
    pub dev: bool,

    /// Only print the planned operation.
    #[arg(long)]
    pub dry_run: bool,

    /// Roll back to the previous installed version.
    #[arg(long, conflicts_with = "dev")]
    pub rollback: bool,

    /// Project directory for --dev builds. Defaults to the current directory.
    #[arg(long)]
    pub project_dir: Option<String>,
}

/// `sebas control` — send a request to the watchdog control plane.
#[derive(Parser)]
pub struct ControlArgs {
    /// Path to the watchdog control socket. Defaults to XDG_RUNTIME_DIR/sebas/control.sock.
    #[arg(long)]
    pub socket: Option<String>,

    #[command(subcommand)]
    pub cmd: ControlCmd,
}

#[derive(Subcommand)]
pub enum ControlCmd {
    /// Ask the watchdog for a control-plane status operation.
    Status,
    /// Print control events after this sequence number.
    Events {
        #[arg(long, default_value_t = 0)]
        since: u64,
    },
    /// Admit an update operation in the watchdog control plane.
    Update {
        /// Build from the configured/local dev target instead of release.
        #[arg(long)]
        dev: bool,
        /// Only plan the update.
        #[arg(long)]
        dry_run: bool,
    },
    /// Admit a rollback operation in the watchdog control plane.
    Rollback {
        /// Only plan the rollback.
        #[arg(long)]
        dry_run: bool,
    },
}
