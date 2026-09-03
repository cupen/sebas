//! WebUI for the sebas daemon: JSON HTTP API, WebSocket realtime channel,
//! and the embedded single-page-application console.

pub mod admin;
pub mod admin_auth;
pub mod api;
pub mod assets;
pub mod events;
pub mod gateway_client;
pub mod models;
pub mod projects;
pub mod routes;
pub mod server;
pub mod session_backend;

#[doc(hidden)]
pub use server::WebUiState;
pub use server::{build_router, run, run_with_admin_adapter};
pub use session_backend::{Reachability, SessionBackend, SessionRejection};
