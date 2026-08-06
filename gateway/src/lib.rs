//! gateway crate：LLM provider router（Anthropic/OpenAI 双协议面纯透传网关）。
//!
//! 本任务（Task 1）仅落地 crate 骨架与 `[gateway]` 配置模型。
//! 协议嗅探、路由、透传引擎、鉴权/限流/用量统计见后续任务。
//!
//! 设计文档：docs/superpowers/specs/2026-08-06-gateway-design.md
//!
//! edition 2021：禁用 let-chains（`if let A = b && cond` 是 2024 语法）。

pub mod config;
pub mod error;
pub mod proto;
