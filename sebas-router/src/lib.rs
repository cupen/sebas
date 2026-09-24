//! router crate：LLM provider router（Anthropic/OpenAI 双协议面纯透传路由）。
//!
//! - Task 1：crate 骨架与 `[router]` 配置模型（`config`/`proto`/`error`）。
//! - Task 2：axum server 启动骨架（`server`：`/healthz` + placeholder fallback）。
//!
//! 协议嗅探、路由、透传引擎、鉴权/用量统计见后续任务。
//!
//! 设计文档：openspec/specs/router-core/spec.md
//!
//! edition 2024：let-chains 可用（`if let A = b && cond`）。

pub mod access_log;
pub mod admin;
/// 确定性 agent-loop 规则（test 模型与 fake 上游共用的规则事实源）。
pub mod agent_loop;
/// Anthropic `/v1/messages` 响应形状构造（test_provider 与 fake_provider 共用）。
pub mod anthropic_wire;
pub mod auth;
pub mod config;
pub mod core_channel;
pub mod debug;
pub mod error;
/// `sebas fake-provider`：本地 Anthropic 线协议假上游（fake-provider-upstream）。
pub mod fake_provider;
pub mod hot_reload;
pub mod key_resolver;
pub mod metrics;
pub mod models;
pub mod probe;
pub mod proto;
pub mod proxy;
pub mod rate_limit;
pub mod routing;
pub mod server;
pub mod sse;
pub mod test_provider;
pub mod usage;

#[cfg(test)]
pub(crate) mod test_util {
    //! 跨模块共享的测试串行锁。`RouterConfig::parse` 读进程 env
    //! （SEBAS_ROUTER_LISTEN / SEBAS_ROUTER_PROVIDER_OVERLAY），任何
    //! set/remove 这些 env 的测试都必须持有同一把锁——历史上 config.rs 与
    //! debug.rs 各持一把 Mutex 产生跨模块竞态（flake）。
    use std::sync::Mutex;

    pub static CONFIG_ENV_LOCK: Mutex<()> = Mutex::new(());

    pub fn lock_config_env() -> std::sync::MutexGuard<'static, ()> {
        CONFIG_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
}
