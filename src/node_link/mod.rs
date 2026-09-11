//! 主控侧的节点链路（add-remote-execution-node）。
//!
//! 本模块承载**主控**这一侧的节点相关状态与（后续的）运行时：
//!
//! - [`registry`]：配对令牌与节点注册表（一次性、带过期、可吊销）；
//! - 运行时（websocket 监听 / 握手）随后落地，与协议类型 crate `sebas-node-link`
//!   共用同一份契约。
//!
//! 权威分域（设计 D1）：注册表只记录**主控需要知道的**东西——节点 id、凭据哈希、
//! 在线态、最后出现时间、吊销。节点自身的执行事实（子进程、turn 日志）不在主控，
//! 也不试图镜像。

pub mod client;
pub mod driver;
pub mod fleet;
pub mod materials;
pub mod placement;
pub mod projection;
pub mod registry;
pub mod runtime;
pub mod server;

pub use client::{NodeConnection, NodeLinkError};
pub use driver::{BatchOutcome, RemoteSession, Segment, Unavailable};
pub use fleet::{ReconcileReport, RemoteFleet, SessionLifecycle};
pub use materials::MaterialStore;
pub use projection::{LOCAL_NODE_ID, Meta0, ProjectionObserver, RemoteProjection};
pub use placement::{
    NO_PROJECT_NAMESPACE, Placed, Placement, PlacementError, ProjectRef, RemoteSessionId,
};
pub use registry::{JoinTokenView, NodeEntry, NodeRegistry, NodeStatus, RegistryError};
pub use runtime::{ArmedNodeLink, arm};
pub use server::{Handled, NodeLinkServer};

use sebas_node_link::RejectCode;

/// 主控侧对一次接入请求的拒绝：机器可判别的码 + 人可读成因。
///
/// 两种用途：注册表内部把「为什么拒绝」返回给调用方；运行时把它翻成
/// [`sebas_node_link::HelloAck`]。这样做是为了**拒绝原因只有一处定义**——
/// 注册表与协议不会各说一套。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    /// 拒绝码（决定节点是否应重试）。
    pub code: RejectCode,
    /// 人类可读成因（会送达节点侧日志与主控侧界面）。
    pub cause: String,
}

impl Rejection {
    /// 构造。
    pub fn new(code: RejectCode, cause: impl Into<String>) -> Self {
        Self {
            code,
            cause: cause.into(),
        }
    }

    /// 翻成协议应答。
    pub fn to_ack(&self) -> sebas_node_link::HelloAck {
        sebas_node_link::rejected(
            sebas_node_link::PROTOCOL_VERSION,
            self.code,
            self.cause.clone(),
        )
    }
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}（{}）", self.cause, self.code.as_str())
    }
}
