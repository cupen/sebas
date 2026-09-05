use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Run the long-lived sebas core service (sessions, adapters, channels).
    Core(CoreArgs),
    /// Install or uninstall the systemd system unit for sebas (requires root).
    Service(ServiceArgs),
    /// Replay captured inbound events from a directory of `.json` files.
    Replay(ReplayArgs),
    /// Record an ACP agent's stdio traffic as a fixture file
    /// （见 openspec/specs/replay-debug/spec.md）.
    /// Type/paste JSON-RPC lines on stdin; responses print to stdout; both
    /// directions are appended to --output as {"dir","msg"} journal lines.
    Record(RecordArgs),
    /// Run the LLM provider router (Anthropic/OpenAI dual-protocol
    /// transparent proxy). See openspec/specs/router-core/spec.md.
    /// 旧名 `gateway` 保留为隐藏别名（rename-cli-surface）。
    #[command(alias = "gateway")]
    Router(RouterArgs),
    /// Start the standalone WebUI dashboard server.
    /// Spawned by the watchdog when `[watchdog.webui] enabled = true`.
    #[command(name = "webui")]
    WebUi(WebUiArgs),
    /// 初始化 / 修改 WebUI 登录账户（用户名 + 密码，PBKDF2 落盘）。
    /// 配置凭据后 webui 全部 API/WS 需登录；非 loopback bind 也只有
    /// 凭据存在时才放行。运行中的 webui 经 mtime 热重载，无需重启。
    #[command(name = "webui-passwd")]
    WebUiPasswd(WebUiPasswdArgs),
    /// Run the watchdog daemon: supervise core/webui/router children and
    /// self-upgrade. 旧名 `watchdog` 保留为隐藏别名（已装 systemd unit 的
    /// ExecStart 仍写 `watchdog --config`，升级后不重装也能启动）。
    #[command(alias = "watchdog")]
    Run(RunArgs),
    /// One-shot update implementation used by watchdog.
    Update(UpdateArgs),
    /// Send a command to the watchdog control plane.
    Control(ControlArgs),
    /// Shorthand for `sebas control status` (control-plane status snapshot).
    Status(ControlStatusArgs),
    /// Shorthand for `sebas control services` (managed-service status snapshot).
    Services(ControlStatusArgs),
    /// Alias for `sebas control` (watchdog control plane).
    Ctl(ControlArgs),
    /// Report reachability of the configured third-party agents.
    #[command(name = "agent-kinds")]
    AgentKinds(AgentKindsArgs),
}

/// Core mode — the long-lived sebas core service.
#[derive(Parser)]
pub struct CoreArgs {
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,

    /// 同时在随机端口（127.0.0.1:0）上启动内置 router；实际端口在日志中输出。
    /// provider 从配置顶层 `[provider.*]` 读取。
    #[arg(long)]
    pub router: bool,

    /// 同时让内置 router 进入 debug 模式：增加 `test` 模型，由 router 自身
    /// 应答（固定文字 + 回显输入），不转发外部上游。
    #[arg(long)]
    pub debug: bool,

    /// Send a startup "sebas 已启动" message to this chat_id, then continue running.
    /// Useful for verifying outbound is wired correctly. chat_id format depends on
    /// receive_id_type: open_id (private) or chat_id (group).
    #[arg(long)]
    pub test_msg: Option<String>,

    /// Start the WebUI dashboard server.
    #[arg(long, conflicts_with = "no_webui")]
    pub webui: bool,

    /// Port for the WebUI server (default: 9797).
    #[arg(long, default_value = "9797")]
    pub webui_port: u16,

    /// Explicitly disable the WebUI dashboard server (symmetry with watchdog
    /// default, no-op in bare run mode).
    #[arg(long, conflicts_with = "webui")]
    pub no_webui: bool,

    /// Dump every raw inbound WS payload to this directory as one .json file per
    /// event (timestamp-prefixed). Useful for local replay/debug without needing
    /// the live Feishu connection. Disabled when omitted.
    #[arg(long)]
    pub dump_inbound: Option<String>,
}

#[derive(Parser)]
pub struct ServiceArgs {
    /// Install the sebas system unit (/etc/systemd/system/sebas.service).
    #[arg(long, conflicts_with = "uninstall", required = true)]
    pub install: bool,

    /// Uninstall the sebas system unit.
    #[arg(long, conflicts_with = "install")]
    pub uninstall: bool,

    /// OS account the service runs as (User=/Group=). Must not be root.
    /// Required for --install (enforced at runtime); ignored by --uninstall.
    #[arg(long, default_value = "")]
    pub user: String,

    /// After installing, also `systemctl enable --now` the unit.
    #[arg(long)]
    pub auto_start: bool,

    /// Overwrite an existing unit file.
    #[arg(long)]
    pub force: bool,

    /// Path to the sebas config.toml to bake into ExecStart. Must be absolute.
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,

    /// Bake a specific RUST_LOG value into the unit. When omitted, the installing
    /// environment's RUST_LOG is inherited (falling back to info). Install-only.
    #[arg(long)]
    pub log_level: Option<String>,
}

#[derive(Parser)]
pub struct ReplayArgs {
    /// Directory containing `.json` files to replay (one neutral channel event per file).
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

/// `sebas router` — run the LLM provider router.
#[derive(Parser)]
pub struct RouterArgs {
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,

    /// 启用 debug 模式：增加内置 `test` 模型，router 自身应答
    /// （固定文字 + 回显输入），不转发外部上游。
    #[arg(long)]
    pub debug: bool,
}

/// `sebas webui` — start the standalone WebUI dashboard server.
/// Spawned by the watchdog when `[watchdog.webui] enabled = true`.
#[derive(Parser)]
pub struct WebUiArgs {
    /// Path to the sebas config.toml.
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,
}

/// `sebas webui-passwd` — create or update the WebUI login account.
#[derive(Parser)]
pub struct WebUiPasswdArgs {
    /// 账户用户名。缺省时保留现有凭据的用户名（首次建户必须提供）。
    #[arg(long)]
    pub user: Option<String>,
    /// 新密码（明文；<8 字符仅告警不拦截——测试环境统一 admin/admin）。
    /// 优先用 --password-stdin，避免密码进入 shell history。
    #[arg(long)]
    pub password: Option<String>,
    /// 从 stdin 读一行作为新密码
    /// （`printf '%s' 'pw' | sebas webui-passwd --password-stdin`）。
    #[arg(long, conflicts_with = "password")]
    pub password_stdin: bool,
}

/// `sebas run` — start the watchdog daemon.
/// Manages the sebas child processes and handles self-upgrade.
#[derive(Parser)]
pub struct RunArgs {
    /// Path to the sebas config.toml.
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,

    /// 同时在固定端口上以 debug 模式额外启动一个独立 router HTTP 服务
    /// （内置 `test` 模型自应答、不转发上游），便于本地 curl 调试。
    #[arg(long)]
    pub debug: bool,
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
    /// Path to the watchdog control socket.
    /// Precedence: --socket > $SEBAS_CONTROL_SOCKET > XDG_RUNTIME_DIR/sebas/control.sock.
    #[arg(long)]
    pub socket: Option<String>,

    /// Control RPC secret for authentication.
    /// Precedence: --secret > $SEBAS_CONTROL_SECRET > error.
    #[arg(long)]
    pub secret: Option<String>,

    /// Output format. `human` is one-line/key-value text; `json` is the raw
    /// `RpcControlResponse` envelope, stable across releases.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human, global = true)]
    pub format: OutputFormat,

    #[command(subcommand)]
    pub cmd: ControlCmd,
}

/// 顶层 `sebas status` / `sebas services` 的轻量参数（无子命令）。
/// 复用 ControlArgs 的 socket/secret/format 解析，只是没有 `cmd`。
#[derive(Parser)]
pub struct ControlStatusArgs {
    /// Path to the watchdog control socket.
    /// Precedence: --socket > $SEBAS_CONTROL_SOCKET > XDG_RUNTIME_DIR/sebas/control.sock.
    #[arg(long)]
    pub socket: Option<String>,

    /// Control RPC secret for authentication.
    /// Precedence: --secret > $SEBAS_CONTROL_SECRET > error.
    #[arg(long)]
    pub secret: Option<String>,

    /// Output format. `human` is one-line/key-value text; `json` is the raw
    /// `RpcControlResponse` envelope, stable across releases.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Human,
    Json,
}

/// Watchdog control-plane subcommands. Phase 6 (sebas-npc) freezes this surface
/// so that WebUI/Feishu/CLI adapters all share the same normalized request.
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
    /// Restart the sebas core child under the watchdog.
    RestartCore,
    /// Print the watchdog's managed-service status snapshot.
    Services,
}

/// `sebas agent-kinds` — reachability reporting for configured third-party
/// agents (openspec/changes/multi-third-party-acp-agents, agent-driver spec).
#[derive(Parser)]
pub struct AgentKindsArgs {
    #[command(subcommand)]
    pub cmd: AgentKindsCmd,
}

#[derive(Subcommand)]
pub enum AgentKindsCmd {
    /// List each configured agent kind with reachability + version.
    List(AgentKindsListArgs),
}

#[derive(Parser)]
pub struct AgentKindsListArgs {
    /// Path to the sebas config.toml.
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,

    /// Output the raw `AgentKindInfo` list as JSON instead of the table.
    #[arg(long)]
    pub json: bool,
}
