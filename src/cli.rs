use clap::{Args, Parser, Subcommand};

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
    Router(RouterArgs),
    /// 本地 Anthropic 线协议假上游（测试与演示用，零 token、确定性应答）。
    /// 只实现 `/v1/messages`；auth header 不校验；`--journal` 落请求留痕。
    /// 仅供 dummy key 的测试上游使用，绝不可指向生产。
    /// See openspec/specs/fake-provider-upstream/spec.md.
    #[command(name = "fake-provider")]
    FakeProvider(FakeProviderArgs),
    /// Start the standalone WebUI dashboard server.
    /// Spawned by the watchdog when `[service.webui] enabled = true`.
    #[command(name = "webui")]
    WebUi(WebUiArgs),
    /// WebUI 账户管理组命令（add / passwd / list；语义按动词拆分，见
    /// openspec/specs/auth-cli/spec.md）。用户库路径取 `SEBAS_WEBUI_AUTH_DB`
    /// env（缺省 ~/.sebas/auth.db），与运行中 webui 同源；不读 config.toml。
    Auth(AuthArgs),
    /// Run the watchdog daemon: supervise core/webui/router children and
    /// self-upgrade.
    Run(RunArgs),
    /// Start the standalone IM service (Feishu bot host). Spawned by the
    /// watchdog when `[watchdog.im] enabled = true`（extract-im-service）。
    #[command(name = "im")]
    Im(ImArgs),
    /// 节点链路管理：签发配对 token / 列出节点 / 吊销（经 core 会话通道）。
    #[command(name = "node-link")]
    NodeLink(NodeLinkArgs),
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
    /// Run the sebas-agent capability benchmark (agent-bench spec).
    #[command(name = "agent-bench")]
    AgentBench(AgentBenchArgs),
    /// 会话外直连飞书的一次性命令：发文本/图片给指定会话（通知/运维）。
    /// 直接使用 `[feishu]` 凭据，不经过任何运行中的 sebas 实例。
    Feishu(FeishuArgs),
    /// 操作者级 skill 仓（agentskills 格式，add-agent-skills 4.1–4.4）：
    /// list / add / remove / sync。add 是社区流程的薄封装（本地目录 /
    /// git URL / npx skills add 三形态分派）；sync 把仓投影到 configured
    /// backends（claude/codex），无落点的 backend 如实报告 no placement。
    Skills(SkillsArgs),
}

/// `sebas feishu` — 会话外飞书交互（通知用户、投递图片等）。
#[derive(Parser)]
pub struct FeishuArgs {
    /// Path to the sebas config.toml（提供 [feishu] 凭据）。
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,

    /// 目标会话 chat_id（`oc_` 开头；p2p 会话 id 见日志里的 open_chat_id）。
    #[arg(long, global = true)]
    pub chat: Option<String>,

    #[command(subcommand)]
    pub cmd: FeishuCmd,
}

#[derive(Subcommand)]
pub enum FeishuCmd {
    /// 发文本消息。
    Text {
        /// 消息内容（多个词会以空格拼接）。
        #[arg(trailing_var_arg = true)]
        message: Vec<String>,
    },
    /// 发本地图片（先上传换取 image_key，再发图片消息）。
    Image {
        /// 图片文件路径（png/jpg 等飞书支持的格式）。
        path: String,
    },
}

/// `sebas skills` 的参数（src/skills_cmd.rs；core 逻辑全在 sebas::skills，
/// 这里只是薄壳的薄壳）。
#[derive(Parser)]
pub struct SkillsArgs {
    /// Path to the sebas config.toml（仓目录 = `[skills] dir`，缺省
    /// `~/.agents/skills`；sync 的投影对象 = `[acp.agents.*]`）。global：
    /// `sebas skills -c <path> list` 与 `sebas skills list -c <path>` 皆可。
    #[arg(short = 'c', long, default_value = "./config.toml", global = true)]
    pub config: String,

    #[command(subcommand)]
    pub cmd: SkillsCmd,
}

/// `sebas skills` 子命令（与 src/skills_cmd.rs 的 SkillsCmd 一一对应）。
#[derive(Subcommand)]
pub enum SkillsCmd {
    /// 列出仓内条目：name / description / invalid 标记（附原因）。仓目录
    /// 缺失按空仓处理，不报错。
    List,
    /// 落仓一个来源：本地目录（校验 SKILL.md 后整目录拷贝）/ git URL
    /// （http(s)、git@ 前缀或 .git 后缀）/ 其余交 `npx skills add`。
    Add {
        /// 来源：本地目录路径、git URL 或 npx 包描述（如 `owner/repo`）。
        source: String,
    },
    /// 只删仓内条目；绝不动任何 backend（backend 里的副本在下一次
    /// `sebas skills sync` 时清理）。不存在即报错。
    Remove {
        /// 仓内条目名（目录名，不是 frontmatter 的 name）。
        name: String,
    },
    /// 把仓投影到 configured backends 的 skill 目录（镜像语义：仓 wins；
    /// 上次投影过、这次仓里已无的条目随删；名外条目是私产，不碰）。
    /// 无落点约定的 backend 如实报告 no placement。
    Sync,
}

/// `sebas agent-bench` — scripted-client capability benchmark.
#[derive(Parser)]
pub struct AgentBenchArgs {
    /// Run only the smoke subset (error_recovery + static_processing).
    #[arg(long)]
    pub smoke: bool,
    /// Print each tool call/result as it runs.
    #[arg(long)]
    pub debug: bool,
    /// Run twice and assert identical event-key sequences.
    #[arg(long)]
    pub replay: bool,
    /// Record per-task event traces to this JSONL file.
    #[arg(long)]
    pub record: Option<String>,
    /// Comma-separated task-id filter (default: all tasks).
    #[arg(long, value_delimiter = ',')]
    pub tasks: Vec<String>,
}

/// Core mode — the long-lived sebas core service.
#[derive(Parser)]
pub struct CoreArgs {
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,

    // unify-router-process-shape 1.1：内嵌 router 退役，`--router`/`--debug`
    // 旗标已删除（传入即 unknown-argument 报错）。需要 router 就跑独立进程：
    // `sebas router --config <path> [--debug]`（手工/watchdog 子进程同一入口）。
    /// Start the WebUI dashboard server.
    #[arg(long, conflicts_with = "no_webui")]
    pub webui: bool,

    /// Port for the WebUI server (default: 9797).
    #[arg(long, default_value = "9797")]
    pub webui_port: u16,

    /// Bind address for the WebUI server (default: 127.0.0.1).
    /// Docker/容器形态传 0.0.0.0，否则发布的端口不可达。
    #[arg(long, default_value = "127.0.0.1")]
    pub webui_host: String,

    /// Explicitly disable the WebUI dashboard server (symmetry with watchdog
    /// default, no-op in bare run mode).
    #[arg(long, conflicts_with = "webui")]
    pub no_webui: bool,
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

/// `sebas fake-provider` — 本地 Anthropic 线协议假上游（fake-provider-upstream）。
#[derive(Parser)]
pub struct FakeProviderArgs {
    /// 监听地址。缺省 `127.0.0.1:0`（系统分配随机端口；实际绑定地址以
    /// stdout 单行 `fake-provider listening addr=…` 为准）。
    #[arg(long, default_value = "127.0.0.1:0")]
    pub listen: String,

    /// 可选 JSON 剧本文件：条目按序消费（文本 / tool_use / 错误注入），
    /// 耗尽后回落内置确定性规则。文件缺失或非法 JSON = 启动失败（退出码 75）。
    #[arg(long)]
    pub scenario: Option<String>,

    /// 可选 NDJSON 请求留痕文件（method / path / headers / body 逐行追加）。
    /// 套件离线断言透传行为（上游 key 注入、下游 key 不泄漏）的数据源。
    /// 明文含 header——只应指向 dummy key 的测试上游。
    #[arg(long)]
    pub journal: Option<String>,
}

/// `sebas webui` — start the standalone WebUI dashboard server.
/// Spawned by the watchdog when `[service.webui] enabled = true`.
#[derive(Parser)]
pub struct WebUiArgs {
    /// Path to the sebas config.toml.
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,
}

/// `sebas auth` 的参数（add-auth-subcommand；子命令语义见 auth-cli spec，
/// 核心在 src/auth_cmd.rs）。
#[derive(Parser)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub cmd: AuthCmd,
}

/// `sebas auth` 子命令：add / passwd / list，语义按动词拆分——`add` 只建户、
/// `passwd` 只改密，无 create-or-update 一体形态（auth-cli spec「auth 组命令
/// 形态」）。
#[derive(Subcommand)]
pub enum AuthCmd {
    /// 建户：同名（大小写不敏感）已存在则报错并提示用 `auth passwd`。
    /// 缺省角色：库零用户 → root，否则 member（`--role` 显式覆盖）。
    Add {
        /// 账户用户名（位置参数）。
        username: String,
        /// 角色（root/admin/member/viewer），显式给出时覆盖缺省规则。
        #[arg(long)]
        role: Option<String>,
        /// 新密码（明文；<8 字符仅告警不拦截——测试环境统一 admin/admin）。
        /// 优先用 --password-stdin，避免密码进入 shell history。
        #[arg(long)]
        password: Option<String>,
        /// 从 stdin 读一行作为密码
        /// （`printf '%s' 'pw' | sebas auth add <name> --password-stdin`）。
        #[arg(long, conflicts_with = "password")]
        password_stdin: bool,
    },
    /// 改密（新盐新哈希）：用户不存在则报错并提示用 `auth add`。不携带
    /// `--role`——角色调整归 WebUI root 管理面。
    Passwd {
        /// 账户用户名（位置参数）。
        username: String,
        /// 新密码（明文；<8 字符仅告警不拦截）。优先用 --password-stdin。
        #[arg(long)]
        password: Option<String>,
        /// 从 stdin 读一行作为密码
        /// （`printf '%s' 'pw' | sebas auth passwd <name> --password-stdin`）。
        #[arg(long, conflicts_with = "password")]
        password_stdin: bool,
    },
    /// 只读列出用户库账户：用户名 / 角色 / 启用 / 时间戳（不含哈希）。
    List,
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

    /// 日志过滤，RUST_LOG 语法：级别（error/warn/info/debug/trace）或
    /// 完整过滤式（如 `info,openlark=debug`）。覆盖 config `[log] level`，
    /// 并注入 core/webui/im 全部子进程——把与飞书的交互（收到的点击/
    /// 消息、路由决策、发送动作）打全用 `--log-level debug`。
    #[arg(long)]
    pub log_level: Option<String>,
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

/// `sebas node-link` 的参数。
#[derive(Args, Debug, Clone)]
pub struct NodeLinkArgs {
    /// 主控配置文件（用于发现 core socket 与 secret）。
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,
    /// 管理操作。
    #[command(subcommand)]
    pub cmd: NodeLinkCmd,
}

/// 节点链路管理操作。
#[derive(Subcommand, Debug, Clone)]
pub enum NodeLinkCmd {
    /// 签发一个一次性配对 token（只显示这一次；节点用它换取长期凭据）。
    Token {
        /// 有效期（秒）。缺省由 core 决定（900）。
        #[arg(long)]
        ttl: Option<u64>,
    },
    /// 列出已注册节点及其在线态。
    List,
    /// 吊销节点凭据：此后不可接入，也不能靠重新配对绕过。
    Revoke {
        /// 节点标识。
        node_id: String,
    },
}

/// `sebas im` 的参数。
#[derive(Args, Debug, Clone)]
pub struct ImArgs {
    #[arg(short = 'c', long, default_value = "./config.toml")]
    pub config: String,
    /// Optional startup test message (sent to this receive_id as chat_id).
    #[arg(long)]
    pub test_msg: Option<String>,
    /// Dump inbound WS payloads to this directory (debug affordance).
    #[arg(long)]
    pub dump_inbound: Option<String>,
    /// 日志过滤（RUST_LOG 语法）。缺省读 RUST_LOG 环境变量，再退 `info`。
    #[arg(long)]
    pub log_level: Option<String>,
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
