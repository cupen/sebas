//! sebas library crate: runtime modules shared by the CLI binary.

/// The long-lived core subcommand. The watchdog spawns its child with exactly
/// this argv (see `watchdog::Watchdog::spawn_child`), and the systemd unit in
/// `service` bakes the same into `ExecStart`. The binary's clap
/// subcommand must stay in sync — rename the `Cmd::Run` variant and this const
/// together (tests in `main.rs` assert the two agree).
pub const CORE_SUBCOMMAND: &str = "run";

pub mod config;
mod dispatch;
pub mod error;
pub mod gateway_cmd;
pub mod service;
pub mod ipc;
pub mod provider;
// `provider_state` 已迁到 router crate（sebas-63f.5 解决 sebas→router 反向依赖）；
// sebas 内部用 `router::provider_state`。
pub mod reactions;
pub mod record;
pub mod replay;
pub mod run;
mod session_boot;
pub mod spawn_env;
pub mod update;
pub mod upgrade;
pub mod watchdog;
pub mod webui_cmd;
mod ws_loop;
