//! gateway crate：LLM provider router（Anthropic/OpenAI 双协议面纯透传网关）。
//!
//! - Task 1：crate 骨架与 `[gateway]` 配置模型（`config`/`proto`/`error`）。
//! - Task 2：axum server 启动骨架（`server`：`/healthz` + placeholder fallback）。
//!
//! 协议嗅探、路由、透传引擎、鉴权/用量统计见后续任务。
//!
//! 设计文档：docs/superpowers/specs/2026-08-06-gateway-design.md
//!
//! edition 2024：let-chains 可用（`if let A = b && cond`）。

pub mod access_log;
pub mod auth;
pub mod config;
pub mod error;
pub mod models;
pub mod proto;
pub mod proxy;
pub mod routing;
pub mod server;
pub mod sse;
pub mod test_provider;
pub mod usage;
