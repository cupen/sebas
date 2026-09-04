//! WebUI for the sebas daemon: JSON HTTP API, WebSocket realtime channel,
//! and the embedded single-page-application console.

pub mod admin;
pub mod admin_auth;
pub mod agent_kinds;
pub mod api;
pub mod archive;
pub mod assets;
pub mod events;
pub mod fs;
pub mod gateway_client;
pub mod models;
pub mod projects;
pub mod routes;
pub mod server;
pub mod session_backend;
pub mod web_adapter;

#[doc(hidden)]
pub use server::WebUiState;
pub use server::{build_router, build_router_with_agent_kind_provider, run, run_with_admin_adapter};
pub use session_backend::{Reachability, SessionBackend, SessionRejection};
