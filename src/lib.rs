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

pub mod config;
pub mod agent_backend;
pub mod agent_kinds;
pub mod core_channel;
mod dispatch;
pub mod error;
pub mod router_cmd;
pub mod ipc;
pub mod native_dispatch_bridge;
pub mod provider;
pub mod service;
// `provider_state` 已迁到 router crate（sebas-63f.5 解决 sebas→router 反向依赖）；
// sebas 内部用 `sebas_dispatch::provider_state`。
pub mod reactions;
pub mod record;
pub mod replay;
pub mod run;
pub mod sebas_state;
mod session_boot;
pub mod spawn_env;
pub mod update;
pub mod upgrade;
pub mod watchdog;
pub mod webui_cmd;
mod ws_loop;
