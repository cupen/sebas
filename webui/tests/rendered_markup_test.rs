//! Invariants that only the *rendered* markup can prove.
//!
//! The endpoint tests check status codes and individual fields. These check
//! properties of every page at once — the kind of thing a status-code test is
//! structurally blind to. Both invariants here started as manual verification
//! steps in the `redesign-webui-console` change; asserting them is strictly
//! better than re-checking them by hand each time a template moves.

use acp_claude::manager::SessionManager;
use axum::body::Body;
use axum::http::Request;
use feishu::events::SessionKey;
use http_body_util::BodyExt;
use router::router::RouterHandle;
use router::state::{Mapping, SessionMap};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use webui::models::GatewayInfo;
use webui::{build_router, init_templates_for_tests};

/// Every GET page in the dashboard, as a (path, label) pair.
const PAGES: &[&str] = &["/", "/sessions", "/settings", "/gateway", "/about"];

fn key(id: &str) -> SessionKey {
    SessionKey {
        chat_id: format!("oc_{id}"),
        thread_id: None,
    }
}

async fn app() -> axum::Router {
    let map = SessionMap::new();
    map.insert(key("a"), Mapping::active("sess-abcdef0123456789-tail"))
        .await
        .unwrap();
    map.insert(key("b"), Mapping::dormant("sess-b", 3))
        .await
        .unwrap();
    map.insert(key("c"), Mapping::spawning()).await.unwrap();

    let (router, _rx) = RouterHandle::new(map);
    router
        .seed_card("sess-abcdef0123456789-tail".into(), "a prompt".into())
        .await;
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(5)));
    let templates = Arc::new(init_templates_for_tests());
    build_router(router, mgr, GatewayInfo::default(), templates)
}

async fn render(app: &axum::Router, path: &str) -> String {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "GET {path} returned {}",
        resp.status()
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The console must render fully with outbound network blocked: every font,
/// script and stylesheet is self-hosted under `/static`. A single CDN `src`
/// reintroduced by a later edit would leave the console degrading silently on
/// an air-gapped host — visible only to whoever runs it there.
#[tokio::test]
async fn no_page_references_an_external_asset() {
    let app = app().await;
    for path in PAGES {
        let body = render(&app, path).await;
        for attr in ["src=\"", "href=\""] {
            for (_, rest) in body.match_indices(attr).map(|(i, m)| (i, &body[i + m.len()..])) {
                let value = rest.split('"').next().unwrap_or("");
                let external = value.starts_with("http://")
                    || value.starts_with("https://")
                    || value.starts_with("//");
                assert!(
                    !external,
                    "{path} references external asset {value:?} — every asset \
                     must be self-hosted under /static"
                );
            }
        }
    }
}

/// The nav and brand chrome live in exactly one template, so navigating must
/// not shift them. Everything up to `<main>` should be byte-identical across
/// pages apart from the title and which nav item is marked current.
#[tokio::test]
async fn shell_chrome_is_identical_across_pages() {
    let app = app().await;
    const MARK: &str = "<main class=\"main-content\"";

    let mut shells = Vec::new();
    for path in PAGES {
        let body = render(&app, path).await;
        let end = body
            .find(MARK)
            .unwrap_or_else(|| panic!("{path} has no <main> landmark"));
        // Normalise the two intended per-page differences away.
        let shell = body[..end]
            .lines()
            .filter(|l| !l.contains("<title>"))
            .map(|l| l.replace("nav-item active", "nav-item").replace(
                "aria-current=\"page\"",
                "",
            ))
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n");
        shells.push((*path, shell));
    }

    let (first_path, first) = &shells[0];
    for (path, shell) in &shells[1..] {
        assert_eq!(
            first, shell,
            "shell chrome differs between {first_path} and {path} beyond the \
             page title and current-page marker"
        );
    }
}
