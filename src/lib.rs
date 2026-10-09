//! sebas library crate: runtime modules shared by the CLI binary.

/// SEBAS_HOME / SEBAS_STATE_DIR 两个 env 变量的**测试互斥锁**
/// （unify-sebas-home）：这两个变量被多个模块在测试里 set/remove（config
/// 缺省、upgrade 缺省、watchdog PinnedEnv……），而 env 是进程全局的——
/// 并行用例必须共用这一把锁，否则 A 用例 set、B 用例 restore 会互相踩。
/// 同 crate 的单元测试（lib 目标）用 [`crate::home_env_test_lock`]；
/// bin 目标（main.rs tests）是独立进程，自持锁即可。
#[cfg(test)]
pub fn home_env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    &LOCK
}

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
/// core→router admin 的 loopback HTTP 反代取数（add-usage-statistics D1/D7）。
pub mod router_admin;
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
/// 本地回合用量账本（add-local-usage-statistics：usage_local.db 的行 struct、
/// 单写 sink、双算规避门控与 source 三口径查询编排）。
pub mod usage_local;
pub mod watchdog;
pub mod webui_cmd;
mod ws_loop;
