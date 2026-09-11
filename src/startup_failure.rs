//! 兼容层：启动失败契约的实现已抽到叶子 crate [`sebas_startup`]，由主控
//! `sebas` 与执行节点 `sebas-node` 两个二进制共用（add-remote-execution-node
//! D0：共享而非复制）。本模块只做再导出，既有的 `crate::startup_failure::*`
//! 调用点保持原位、零改动。
//!
//! 契约正文见 [`sebas_startup`] 的模块文档。
pub use sebas_startup::*;
