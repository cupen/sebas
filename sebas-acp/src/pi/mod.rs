//! pi 驱动（add-pi-driver）：`pi --mode rpc` 子进程的第三个一等
//! [`crate::AgentDriver`] 实现。
//!
//! - [`codec`]：协议编解码与事件/命令翻译（纯函数，录制帧 fixture 锁形状）；
//! - [`driver`]：[`PiDriver`] 进程编排（握手、双泵、事件/命令回路、恢复
//!   语义、无权限系统的如实呈现）。
//!
//! pi 协议类型不出本模块边界——对外只产出 [`crate::session`] 的统一
//! `AcpEvent`/`AcpCommand` 词表。

mod codec;
pub mod driver;

pub use driver::{DEFAULT_PI_SESSIONS_DIR, PiDriver};
