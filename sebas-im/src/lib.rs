//! sebas-im：IM 服务层（extract-im-service M1 起步）。
//!
//! 目标形态（openspec/changes/extract-im-service）：feishu 及未来其它 IM 的
//! 适配器宿主与交互状态机（卡片、审批卡、命令、表单、reactions、媒体）独立
//! 成库与服务进程（`sebas im`），经核心会话通道观察并驱动会话；core 二进制
//! 不再链接任何 IM 实现。
//!
//! M1（crate 内聚阶段）：本 crate 先承接与 sebas-dispatch 无耦合的 IM 域
//! 代码 —— reactions 记账（自 src/reactions.rs 迁入）与飞书装配入口
//! （自 src/run.rs 的 `sebas core` 装配段抽出）；卡片机/命令/表单等在
//! M3 割接时随行为切换一并迁入（保持单向依赖 sebas-dispatch，避免环）。

pub mod bootstrap;
pub mod frontend;
pub mod media;
pub mod port;
pub mod reactions;

pub use reactions::{ReactPlan, ReactionTracker};
