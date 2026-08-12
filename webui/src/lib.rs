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

pub use server::{build_router, run};
#[doc(hidden)]
pub use server::{init_templates_for_tests, WebUiState};