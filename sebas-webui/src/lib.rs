//! WebUI dashboard for the sebas daemon.
//!
//! Provides a management dashboard with:
//! - Dashboard overview (active sessions, uptime, status)
//! - Session list and detail views
//! - Configuration display
//! - Real-time SSE event stream

pub mod admin;
pub mod admin_auth;
pub mod gateway_client;
pub mod models;
pub mod routes;
pub mod server;
pub mod session_backend;
pub mod sse;

#[doc(hidden)]
pub use server::{WebUiState, init_templates_for_tests};
pub use session_backend::{Reachability, SessionBackend, SessionRejection};
pub use server::{build_router, run, run_with_admin_adapter};
