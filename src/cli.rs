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
}

/// Run mode — the long-lived sebas service.
#[derive(Parser)]
pub struct RunArgs {
    #[arg(long, default_value = "./config.toml")]
    pub config: String,

    /// Send a startup "sebas 已启动" message to this chat_id, then continue running.
    /// Useful for verifying outbound is wired correctly. chat_id format depends on
    /// receive_id_type: open_id (private) or chat_id (group).
    #[arg(long)]
    pub test_msg: Option<String>,

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
    #[arg(long, default_value = "./config.toml")]
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
    #[arg(long, default_value = "./config.toml")]
    pub config: String,

    /// Extra args for the agent binary, after `--`
    /// (appended to the configured acp.claude.args).
    #[arg(last = true)]
    pub agent_args: Vec<String>,
}
