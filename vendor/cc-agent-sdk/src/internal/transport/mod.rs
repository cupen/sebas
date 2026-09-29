//! Transport layer for communicating with Claude Code CLI

pub mod pooled;
pub mod subprocess;
mod trait_def;

// PooledTransport is used directly from crate::internal::transport::pooled
// in client.rs rather than through this module
pub use subprocess::{BufferMetricsSnapshot, SubprocessTransport};
pub use trait_def::Transport;
