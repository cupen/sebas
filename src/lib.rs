//! sebas library crate: runtime modules shared by the CLI binary.

/// The long-lived core subcommand (`sebas core`). The watchdog spawns its
/// child with exactly this argv (see `watchdog::CoreSpawner`). The binary's
/// clap subcommand must stay in sync — rename the `Cmd::Core` variant and
/// this const together (tests in `main.rs` assert the two agree).
pub const CORE_SUBCOMMAND: &str = "core";

/// The watchdog-daemon subcommand (`sebas run`). The systemd unit in
/// `service` bakes this into `ExecStart` (the supervisor is the thing systemd
/// actually runs). `watchdog` survives as a hidden clap alias so
/// already-installed units keep booting across the rename.
pub const RUN_SUBCOMMAND: &str = "run";

/// The model-router subcommand (`sebas router`; hidden alias `gateway`).
pub const ROUTER_SUBCOMMAND: &str = "router";

pub mod agent_backend;
pub mod agent_kinds;
/// Agent 目录的 store 侧 glue（add-agent-settings-and-session-titles）：
/// config 种子导入、spawn 动态解析与注册表登记。
pub mod agent_store;
/// 会话自动标题的触发编排（add-agent-settings-and-session-titles 6.2）。
pub mod auto_title;
/// `sebas auth` 命令行面（add-auth-subcommand；建户/改密/列表核心在本模块，
/// 存储层在 `sebas_webui::user_store`）。
pub mod auth_cmd;
pub mod config;
pub mod core_channel;
mod dispatch;
pub mod error;
pub mod fake_provider_cmd;
pub mod feishu_cmd;
pub mod im_cmd;
pub mod ipc;
pub mod native_dispatch_bridge;
pub mod node_link;
pub mod node_link_cmd;
pub mod provider;
pub mod router_cmd;
pub mod service;
/// 操作者级 skill 仓 + 多 backend 方言投影（add-agent-skills）。
pub mod skills;
/// `sebas skills` 命令行面（add-agent-skills 4.1–4.4；core 逻辑在
/// [`skills`]，这里是薄壳）。
pub mod skills_cmd;
// `provider_state` 已迁到 router crate（sebas-63f.5 解决 sebas→router 反向依赖）；
// sebas 内部用 `sebas_dispatch::provider_state`。
pub mod record;
pub mod replay;
pub mod run;
pub mod sebas_state;
mod session_boot;
pub mod spawn_env;
pub mod startup_failure;
pub mod update;
pub mod upgrade;
pub mod watchdog;
pub mod webui_cmd;
mod ws_loop;
