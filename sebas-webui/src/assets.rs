//! Embedded frontend assets and SPA delivery.
//!
//! The built SPA (`frontend/dist`, built automatically by `build.rs` during
//! `cargo build`) is compiled into the binary so a single `sebas` artifact
//! ships the whole UI: `/assets/*` serves the content-hashed bundle files,
//! and every page path falls back to the SPA entry document so client-side
//! routes survive deep links and reloads.

use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/frontend/dist"]
struct FrontendAssets;

fn lookup(path: &str) -> Option<Response> {
    let file = FrontendAssets::get(path)?;
    let mime = mime_type(path);
    let mut resp = (StatusCode::OK, file.data).into_response();
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, mime.parse().unwrap());
    Some(resp)
}

/// GET /assets/{*path} — content-hashed bundle files, immutable.
pub async fn asset(Path(path): Path<String>) -> Response {
    match lookup(&format!("assets/{path}")) {
        Some(mut resp) => {
            resp.headers_mut()
                .insert(header::CACHE_CONTROL, "public, max-age=31536000, immutable".parse().unwrap());
            resp
        }
        None => (StatusCode::NOT_FOUND, "asset not found").into_response(),
    }
}

/// Favicon and other root-level static files that ship with the bundle.
pub async fn root_file(Path(path): Path<String>) -> Response {
    lookup(&path).unwrap_or_else(|| (StatusCode::NOT_FOUND, "not found").into_response())
}

fn entry_document() -> Response {
    match FrontendAssets::get("index.html") {
        Some(file) => Html(String::from_utf8_lossy(&file.data).into_owned()).into_response(),
        None => (StatusCode::NOT_FOUND, "SPA entry missing").into_response(),
    }
}

/// GET / — the SPA entry document.
pub async fn index() -> Response {
    entry_document()
}

/// Serve the SPA entry for any unmatched GET page path (client-side route
/// fallback). Non-GET and API-ish prefixes stay JSON 404s.
pub async fn spa_fallback(req: axum::extract::Request) -> Response {
    let path = req.uri().path().to_string();
    if req.method() != axum::http::Method::GET
        || path.starts_with("/api/")
        || path == "/api"
        || path.starts_with("/assets/")
        || path.starts_with("/static/")
    {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match lookup("index.html") {
        Some(_) => entry_document(),
        None => (StatusCode::NOT_FOUND, "SPA entry missing").into_response(),
    }
}

fn mime_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("mjs") => "text/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("map") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
