//! Core session channel: the Unix-socket protocol that makes the core the
//! single session authority (openspec/changes/add-core-session-channel).
//!
//! - [`protocol`]: wire types (newline-delimited JSON frames).
//! - [`server`]: the core-side server over the live `RouterHandle`.
//! - [`client`]: the standalone-WebUI-side `SessionBackend` implementation.

pub mod client;
pub mod protocol;
pub mod server;

pub use server::{default_socket_path, socket_path};

#[cfg(test)]
mod tests;
