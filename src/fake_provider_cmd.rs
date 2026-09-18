//! `sebas fake-provider --listen <addr> [--scenario <file>] [--journal <file>]`
//! — 本地 Anthropic 线协议假上游（fake-provider-upstream 2.1）。
//!
//! 与 `sebas router` 同模式：主 crate 只做薄壳，实现全在
//! [`sebas_router::fake_provider`]。scenario 加载失败/端口 bind 失败即启动
//! 失败（EX_TEMPFAIL 75 + `startup-failure:` 末行，见
//! fail-fast-on-startup-errors）。
//!
//! 安全约束：journal 会明文记录收到的 header——本服务只应用于 dummy key 的
//! 测试上游，绝不可把真实 provider 指向它。

use std::path::PathBuf;

use sebas_router::fake_provider::{self, FakeProviderConfig};

use crate::error::{Result, SebasError};

/// Arguments for `sebas fake-provider`.
pub struct FakeProviderArgs {
    pub listen: String,
    pub scenario: Option<String>,
    pub journal: Option<String>,
}

/// CLI entry: build the config, hand off to the router crate, map errors to
/// the root crate's error type (startup-failure semantics live in main.rs).
pub async fn run(args: FakeProviderArgs) -> Result<()> {
    init_tracing();
    let cfg = FakeProviderConfig {
        listen: args.listen,
        scenario: args.scenario.map(PathBuf::from),
        journal: args.journal.map(PathBuf::from),
    };
    fake_provider::run(cfg)
        .await
        .map_err(|e| SebasError::Router(e.to_string()))?;
    Ok(())
}

/// Same filter policy as `router_cmd::init_tracing`（RUST_LOG 优先，缺省 info）。
/// `try_init` 失败（已有全局 subscriber）被忽略：第一个调用者生效。
fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_env("RUST_LOG")
        .unwrap_or_else(|_| EnvFilter::new(crate::config::DEFAULT_LOG_FILTER));
    let _ = fmt().with_env_filter(filter).try_init();
}
