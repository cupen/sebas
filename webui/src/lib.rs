//! WebUI dashboard for the sebas daemon.
//!
//! Provides a management dashboard with:
//! - Dashboard overview (active sessions, uptime, status)
//! - Session list and detail views
//! - Configuration display
//! - Real-time SSE event stream

pub mod models;
pub mod routes;
pub mod server;
pub mod sse;

pub use server::run;