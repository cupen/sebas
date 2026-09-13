//! Integration tests for the JSON API surface (`/api/*`): summary, session
//! list/detail, settings/router/about, and the session mutations with the
//! unified `{ "error": ... }` envelope. Drives the router in-process via
//! axum's `oneshot` — no live listener required.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_channels::ChannelKey;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::{Mapping, SessionMap};
use sebas_feishu::cards::CardConfig;
use sebas_webui::build_router;
use sebas_webui::models::RouterInfo;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

fn key(id: &str) -> ChannelKey {
    ChannelKey::feishu(&format!("oc_{id}"), None)
}

fn encode(key: &ChannelKey) -> String {
    urlencoding::encode(&format!("{}\0{}", key.channel.as_str(), key.reference)).into_owned()
}

/// DispatchHandle preloaded with one Active (s1), one Dormant (s2), one
/// Spawning (s3) session, plus the axum app wired against the in-process
/// backend seam. The second element is the outbound receiver: keeping it
/// alive prevents `DispatchHandle::emit`'s closed-channel debug assertion
/// from firing when a test drives the create/message mutations.
async fn fixture() -> (
    DispatchHandle,
    tokio::sync::mpsc::Receiver<sebas_dispatch::engine::Out>,
    axum::Router,
) {
    let map = SessionMap::new();
    let k1 = key("a");
    let k2 = key("b");
    let k3 = key("c");
    map.insert(k1.clone(), Mapping::active("s1")).await.unwrap();
    map.insert(k2.clone(), Mapping::dormant("s2", 1))
        .await
        .unwrap();
    map.insert(k3.clone(), Mapping::spawning()).await.unwrap();

    let (router, rx) = DispatchHandle::new(map);
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(router.clone()),
    );
    let app = build_router(backend, RouterInfo::default(), CardConfig::default());
    (router, rx, app)
}

async fn request(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<String>,
) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(b) => builder
            .header("content-type", "application/json")
            .body(Body::from(b))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let v = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("non-JSON response from {uri} [{status}]: {text:?}: {e}"));
    (status, v)
}

async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value =
        serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("non-JSON body from {uri}: {e}"));
    (status, v)
}

#[tokio::test]
async fn summary_returns_counts_uptime_and_rows() {
    let (_router, _rx, app) = fixture().await;
    let (status, v) = get_json(&app, "/api/summary").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["active_count"], 1);
    assert_eq!(v["dormant_count"], 1);
    assert_eq!(v["spawning_count"], 1);
    assert_eq!(v["total_sessions"], 3);
    assert!(v["uptime"].as_str().is_some(), "uptime missing: {v}");
    let rows = v["recent_sessions"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        assert!(row["encoded_key"].as_str().is_some());
        assert!(row["status_label"].as_str().is_some());
        assert!(row["status_slug"].as_str().is_some());
        assert!(row["status_glyph"].as_str().is_some());
        assert!(row["last_active"].as_str().is_some());
    }
    // No focus has been set yet.
    assert!(v["active_session_key"].is_null());
}

#[tokio::test]
async fn sessions_list_is_active_first_with_status_projection() {
    let (_router, _rx, app) = fixture().await;
    // Focus the active session first: the contract is focused-first, then
    // most-recent activity.
    let encoded_a = encode(&key("a"));
    let (status, _) = request(
        &app,
        "POST",
        &format!("/api/sessions/{encoded_a}/switch"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, v) = get_json(&app, "/api/sessions").await;
    assert_eq!(
        v["active_session_key"],
        encoded_a.as_str(),
        "focus must be set"
    );
    let rows = v["recent_sessions"].as_array().unwrap();
    let first = rows[0]["reference"].as_str().unwrap();
    assert_eq!(first, "oc_a", "focused session must sort first: {v}");
    assert_eq!(rows[0]["is_active"], true);
    // The others are not focused; recency puts the dormant fixture (ts=1)
    // after everything created "just now".
    let later: Vec<&str> = rows[1..]
        .iter()
        .map(|r| r["reference"].as_str().unwrap())
        .collect();
    assert!(later.contains(&"oc_b"), "dormant fixture missing: {v}");
    // Backend-owned status projection: slug in the known set, matching
    // label, and a distinct glyph (shape channel, not colour-only).
    let slugs: Vec<&str> = rows
        .iter()
        .map(|r| r["status_slug"].as_str().unwrap())
        .collect();
    for slug in &slugs {
        assert!(
            ["starting", "queued", "working", "done", "failed", "dormant"].contains(slug),
            "unknown status slug {slug}"
        );
    }
    assert_eq!(
        rows[0]["status_slug"], "queued",
        "active without phase reads Queued"
    );
    // Numeric recency order: the spawning fixture (just created) precedes
    // the dormant one (timestamp 1), regardless of rendered "…d ago" text.
    assert_eq!(rows[1]["status_slug"], "starting");
    assert_eq!(rows[2]["status_slug"], "dormant");
    assert_ne!(rows[0]["status_glyph"], rows[1]["status_glyph"]);
}
#[tokio::test]
async fn session_detail_returns_payload_and_sets_focus() {
    let (_router, _rx, app) = fixture().await;
    let encoded = encode(&key("a"));
    let (status, v) = get_json(&app, &format!("/api/sessions/{encoded}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["reference"], "oc_a");
    assert_eq!(v["session_id"], "s1");
    assert!(v["status_slug"].as_str().is_some());
    assert!(
        v["entries"].is_array(),
        "conversation entries must be a list: {v}"
    );
    // workbench-conversation-view 1.2：user_prompt / body 退役——旧字段出现
    // 即失败。
    assert!(
        v.get("body").is_none(),
        "retired body field must be gone: {v}"
    );
    assert!(
        v.get("user_prompt").is_none(),
        "retired user_prompt field must be gone: {v}"
    );
    assert!(v["last_active"].as_str().is_some());
    assert_eq!(v["encoded_key"], encoded.as_str());

    // The read focuses the session — a display pointer only.
    let (_, summary) = get_json(&app, "/api/summary").await;
    assert_eq!(summary["active_session_key"], encoded.as_str());
}

#[tokio::test]
async fn session_detail_rejects_invalid_and_unknown_keys() {
    let (_router, _rx, app) = fixture().await;
    // A key with no embedded NUL separator cannot decode.
    let (status, v) = get_json(&app, "/api/sessions/notakey").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().is_some(), "error envelope missing: {v}");

    let encoded = encode(&key("zz"));
    let (status, v) = get_json(&app, &format!("/api/sessions/{encoded}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn settings_router_about_expose_page_data() {
    let (_router, _rx, app) = fixture().await;
    let (status, v) = get_json(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["card_config"].is_object(), "card_config missing: {v}");
    assert!(v["card_config"]["theme_color"].as_str().is_some());
    assert!(v["router"].is_object());

    let (status, v) = get_json(&app, "/api/router").await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["router"].is_object());
    assert!(v["router"]["provider_count"].is_u64());

    let (status, v) = get_json(&app, "/api/about").await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["version"].as_str().is_some());
    assert!(v["uptime"].as_str().is_some());
    assert!(v["provider_count"].is_u64());
}

#[tokio::test]
async fn create_session_returns_201_with_key() {
    let (_router, _rx, app) = fixture().await;
    let (status, v) = request(
        &app,
        "POST",
        "/api/sessions",
        Some(r#"{"prompt": "hello", "agent": "claude"}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body: {v}");
    let key = v["key"].as_str().expect("created key missing");
    // The key round-trips: it appears in the list.
    let (_, list) = get_json(&app, "/api/sessions").await;
    let keys: Vec<&str> = list["recent_sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["encoded_key"].as_str().unwrap())
        .collect();
    assert!(keys.contains(&key), "created session missing from list");
}

#[tokio::test]
async fn send_message_and_error_envelope() {
    let (_router, _rx, app) = fixture().await;
    let encoded = encode(&key("a"));
    let (status, v) = request(
        &app,
        "POST",
        &format!("/api/sessions/{encoded}/message"),
        Some(r#"{"message": "hi"}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"], "ok");

    // Invalid key → 400 with the error envelope.
    let (status, v) = request(
        &app,
        "POST",
        "/api/sessions/notakey/message",
        Some(r#"{"message": "hi"}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn close_session_semantics_over_json() {
    let (_router, _rx, app) = fixture().await;
    // Unknown key → 404, nothing mutated.
    let (status, v) = request(
        &app,
        "POST",
        &format!("/api/sessions/{}/close", encode(&key("zz"))),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(v["error"].as_str().is_some());

    // Dormant mapping drops without a kill.
    let (status, v) = request(
        &app,
        "POST",
        &format!("/api/sessions/{}/close", encode(&key("b"))),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"], "closed");

    let (_, list) = get_json(&app, "/api/sessions").await;
    let keys: Vec<&str> = list["recent_sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["reference"].as_str().unwrap())
        .collect();
    assert!(!keys.contains(&"oc_b"), "closed session still listed");
}

#[tokio::test]
async fn switch_session_returns_route_and_focuses() {
    let (_router, _rx, app) = fixture().await;
    let encoded = encode(&key("b"));
    let (status, v) = request(
        &app,
        "POST",
        &format!("/api/sessions/{encoded}/switch"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["redirect"], format!("/sessions/{encoded}"));

    let (_, summary) = get_json(&app, "/api/summary").await;
    assert_eq!(summary["active_session_key"], encoded.as_str());

    // Unknown key → 404 so the client never navigates to a dead view.
    let (status, v) = request(
        &app,
        "POST",
        &format!("/api/sessions/{}/switch", encode(&key("zz"))),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn mutations_reject_non_post_with_405() {
    let (_router, _rx, app) = fixture().await;
    // Note: GET /api/sessions is the *list* endpoint and stays 200; only the
    // mutation-only paths reject non-POST.
    for uri in [
        "/api/sessions/oc_a/close",
        "/api/sessions/oc_a/switch",
        "/api/sessions/oc_a/message",
    ] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED, "GET {uri}");
    }
}

// fix-webui-detached-status 2.2：/api/settings 的 provider 真源行为。
// fake backend 注入 state_snapshot("providers")，断言三种情形的响应形状。
mod provider_source {
    use super::*;

    fn fake_app(backend: Arc<dyn sebas_webui::SessionBackend>) -> axum::Router {
        build_router(backend, RouterInfo::default(), CardConfig::default())
    }

    #[tokio::test]
    async fn providers_from_state_store_are_served() {
        let fake = sebas_webui::session_backend::FakeBackend::new();
        fake.set_state_domain(
            "providers",
            Some(serde_json::json!({
                "version": 2,
                "providers": {
                    "anthropic": {"id": "anthropic", "base_url_anthropic": "https://api.anthropic.com"}
                },
                "deleted": []
            })),
        );
        let app = fake_app(Arc::new(fake));
        let (status, v) = get_json(&app, "/api/settings").await;
        assert_eq!(status, StatusCode::OK);
        let gw = &v["router"];
        assert_eq!(gw["providers_available"], true);
        assert_eq!(gw["provider_count"], 1);
        assert_eq!(gw["providers"][0]["name"], "anthropic");
    }

    #[tokio::test]
    async fn state_store_error_reports_unavailable_not_empty() {
        let fake = sebas_webui::session_backend::FakeBackend::new();
        fake.set_state_domain(
            "providers",
            Some(serde_json::json!({"error": "state store 未初始化"})),
        );
        let app = fake_app(Arc::new(fake));
        let (_, v) = get_json(&app, "/api/settings").await;
        assert_eq!(v["router"]["providers_available"], false);
        assert_eq!(v["router"]["providers"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn missing_domain_reports_unavailable() {
        // backend 对该域返回 None（如通道断开）→ 同样如实标注不可用。
        let fake = sebas_webui::session_backend::FakeBackend::new();
        let app = fake_app(Arc::new(fake));
        let (_, v) = get_json(&app, "/api/settings").await;
        assert_eq!(v["router"]["providers_available"], false);
        assert_eq!(v["router"]["providers"], serde_json::json!([]));
    }
}

// add-webui-multiuser-rbac 3.2：首启 setup、me 的 needs_setup/role、
// 双字段登录形态，以及 /api/users 全套端点的成功 / 越权 / 保护规则
// （design D4/D5/D6）。带鉴权的 router + tempdir auth.db（小迭代数提速），
// 无监听端口、不触真实 ~/.sebas。
mod multiuser_rbac {
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use sebas_feishu::cards::CardConfig;
    use sebas_webui::auth::AuthHandle;
    use sebas_webui::build_router_with_auth;
    use sebas_webui::models::RouterInfo;
    use sebas_webui::rbac::Role;
    use serde_json::Value;
    use std::net::{IpAddr, SocketAddr};
    use std::sync::Arc;
    use tower::ServiceExt;

    fn test_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 12345)
    }

    /// 零用户 app（needs_setup 形态）。
    async fn fresh_app() -> (axum::Router, tempfile::TempDir, Arc<AuthHandle>) {
        let dir = tempfile::tempdir().unwrap();
        let auth = Arc::new(AuthHandle::open_with_iterations(
            dir.path().join("auth.db"),
            1000,
        ));
        let app = build_router_with_auth(
            Arc::new(sebas_webui::session_backend::FakeBackend::new()),
            RouterInfo::default(),
            CardConfig::default(),
            None,
            Arc::new(sebas_webui::agent_kinds::ConfigAgentKindProvider::new(
                Vec::new(),
            )),
            30,
            auth.clone(),
        );
        (app, dir, auth)
    }

    /// 四角色夹具：root alice + admin ada + member bob + viewer vic。
    async fn rbac_app() -> (axum::Router, tempfile::TempDir, Arc<AuthHandle>) {
        let (app, dir, auth) = fresh_app().await;
        auth.setup_root("alice", "password8").await.unwrap();
        let store = auth.user_store().unwrap();
        store.create("ada", "password8", Role::Admin).unwrap();
        store.create("bob", "password8", Role::Member).unwrap();
        store.create("vic", "password8", Role::Viewer).unwrap();
        (app, dir, auth)
    }

    async fn request(
        app: &axum::Router,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        body: Option<String>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:12345")
            .extension(ConnectInfo(test_addr()));
        if let Some(c) = cookie {
            builder = builder.header("cookie", c);
        }
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let req = builder
            .body(Body::from(body.unwrap_or_default()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|e| panic!("non-JSON body from {uri} [{status}]: {e}"))
        };
        (status, v)
    }

    /// 走登录端点换会话 cookie（`sebas_webui_session=…`，取 Set-Cookie 值）。
    async fn login_cookie(app: &axum::Router, username: &str, password: &str) -> String {
        let req = Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header("host", "127.0.0.1:12345")
            .extension(ConnectInfo(test_addr()))
            .header("content-type", "application/json")
            .body(Body::from(format!(
                r#"{{"username":"{username}","password":"{password}"}}"#
            )))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "login {username}");
        resp.headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .trim()
            .to_string()
    }

    // ── 首启 setup（design D4，spec「首启 root 引导」）──

    #[tokio::test]
    async fn setup_creates_root_and_session_then_conflicts() {
        let (app, _dir, _auth) = fresh_app().await;

        // me：零用户 → needs_setup。
        let (status, v) = request(&app, "GET", "/api/auth/me", None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["needs_setup"], true, "{v}");
        assert_eq!(v["authenticated"], false);

        // 弱密码 → 400，库保持零用户。
        let (status, v) = request(
            &app,
            "POST",
            "/api/auth/setup",
            None,
            Some(r#"{"username":"cupen","password":"short"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
        let (_, v) = request(&app, "GET", "/api/auth/me", None, None).await;
        assert_eq!(v["needs_setup"], true, "弱密码不得留下半初始化状态");

        // 缺字段 → 400。
        let (status, _) = request(
            &app,
            "POST",
            "/api/auth/setup",
            None,
            Some(r#"{"username":"cupen"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // 成功：响应携带会话 cookie，me 报已认证 root。
        let req = Request::builder()
            .method("POST")
            .uri("/api/auth/setup")
            .header("host", "127.0.0.1:12345")
            .extension(ConnectInfo(test_addr()))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"username":"cupen","password":"long-enough"}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cookie = resp
            .headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .trim()
            .to_string();
        assert!(cookie.starts_with("sebas_webui_session="), "{cookie}");

        let (_, v) = request(&app, "GET", "/api/auth/me", Some(&cookie), None).await;
        assert_eq!(v["authenticated"], true, "{v}");
        assert_eq!(v["username"], "cupen");
        assert_eq!(v["role"], "root");
        assert_eq!(v["needs_setup"], false);

        // 非零用户：一律 409。
        let (status, v) = request(
            &app,
            "POST",
            "/api/auth/setup",
            None,
            Some(r#"{"username":"second","password":"long-enough"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
    }

    #[tokio::test]
    async fn setup_is_rejected_when_auth_disabled() {
        // 开关关闭（disabled handle）：无设置页形态 → 400（spec：开关关闭
        // 时完全放行且不触发引导）。
        let app = build_router_with_auth(
            Arc::new(sebas_webui::session_backend::FakeBackend::new()),
            RouterInfo::default(),
            CardConfig::default(),
            None,
            Arc::new(sebas_webui::agent_kinds::ConfigAgentKindProvider::new(
                Vec::new(),
            )),
            30,
            Arc::new(AuthHandle::disabled()),
        );
        let (status, v) = request(
            &app,
            "POST",
            "/api/auth/setup",
            None,
            Some(r#"{"username":"cupen","password":"long-enough"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    }

    // ── 登录形态（design D5）──

    #[tokio::test]
    async fn login_only_accepts_username_password_form() {
        let (app, _dir, auth) = fresh_app().await;
        auth.setup_root("alice", "password8").await.unwrap();

        // 旧 {"secret"} 单字段形态 → 400（缺 username/password 字段）。
        let (status, v) = request(
            &app,
            "POST",
            "/api/auth/login",
            None,
            Some(r#"{"secret":"whatever"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");

        // 凭据错误统一 401。
        let (status, _) = request(
            &app,
            "POST",
            "/api/auth/login",
            None,
            Some(r#"{"username":"alice","password":"wrong"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // 成功登录建立会话。
        let (status, v) = request(
            &app,
            "POST",
            "/api/auth/login",
            None,
            Some(r#"{"username":"alice","password":"password8"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["username"], "alice");
    }

    // ── 用户管理（design D6，spec「用户管理（root 专用）」）──

    #[tokio::test]
    async fn users_surface_is_root_only() {
        let (app, _dir, _auth) = rbac_app().await;
        let alice = login_cookie(&app, "alice", "password8").await;
        let ada = login_cookie(&app, "ada", "password8").await;
        let bob = login_cookie(&app, "bob", "password8").await;
        let vic = login_cookie(&app, "vic", "password8").await;

        // 匿名 401；member/admin/viewer 全操作 403（已认证仍拒）。
        let (status, _) = request(&app, "GET", "/api/users", None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        for (who, cookie) in [("admin", &ada), ("member", &bob), ("viewer", &vic)] {
            let (status, _) = request(&app, "GET", "/api/users", Some(cookie), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{who} 列表必须 403");
            let (status, _) = request(
                &app,
                "POST",
                "/api/users",
                Some(cookie),
                Some(r#"{"username":"x","password":"password8","role":"member"}"#.into()),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{who} 建户必须 403");
        }

        // root：列表 200，形状无哈希字段。
        let (status, v) = request(&app, "GET", "/api/users", Some(&alice), None).await;
        assert_eq!(status, StatusCode::OK);
        let users = v["users"].as_array().expect("users array");
        assert_eq!(users.len(), 4);
        let first = &users[0];
        for field in [
            "id",
            "username",
            "role",
            "enabled",
            "created_at_unix",
            "updated_at_unix",
        ] {
            assert!(first.get(field).is_some(), "缺字段 {field}: {first}");
        }
        let raw = serde_json::to_string(&v).unwrap();
        for leak in ["salt", "hash", "iterations"] {
            assert!(!raw.contains(leak), "列表泄漏 {leak}: {raw}");
        }
    }

    #[tokio::test]
    async fn root_creates_user_who_can_login_immediately() {
        let (app, _dir, _auth) = rbac_app().await;
        let alice = login_cookie(&app, "alice", "password8").await;

        // 用户名撞车（大小写不敏感）→ 409。
        let (status, v) = request(
            &app,
            "POST",
            "/api/users",
            Some(&alice),
            Some(r#"{"username":"ALICE","password":"password8","role":"member"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");

        // 角色词表外 → 400。
        let (status, _) = request(
            &app,
            "POST",
            "/api/users",
            Some(&alice),
            Some(r#"{"username":"carol","password":"password8","role":"boss"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // 成功创建 member：201 + user 形状（无哈希字段），立即能登录。
        let (status, v) = request(
            &app,
            "POST",
            "/api/users",
            Some(&alice),
            Some(r#"{"username":"carol","password":"password8","role":"member"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
        assert_eq!(v["user"]["username"], "carol");
        assert_eq!(v["user"]["role"], "member");
        assert_eq!(v["user"]["enabled"], true);
        assert!(v["user"].get("salt").is_none() && v["user"].get("hash").is_none());

        let cookie = login_cookie(&app, "carol", "password8").await;
        let (status, _) = request(&app, "GET", "/api/summary", Some(&cookie), None).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn password_reset_and_disable_kick_sessions() {
        let (app, _dir, auth) = rbac_app().await;
        let alice = login_cookie(&app, "alice", "password8").await;
        let bob = login_cookie(&app, "bob", "password8").await;
        let bob_id = auth
            .user_store()
            .unwrap()
            .get_by_username("bob")
            .unwrap()
            .unwrap()
            .id;

        // 重置密码 → bob 既有会话立即 401；新密码可登录。
        let (status, v) = request(
            &app,
            "POST",
            &format!("/api/users/{bob_id}/password"),
            Some(&alice),
            Some(r#"{"password":"new-pass-99"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let (status, _) = request(&app, "GET", "/api/summary", Some(&bob), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "重置密码必须踢会话");
        let new_cookie = login_cookie(&app, "bob", "new-pass-99").await;
        let (status, _) = request(&app, "GET", "/api/summary", Some(&new_cookie), None).await;
        assert_eq!(status, StatusCode::OK);

        // 禁用 → 既有会话 401（spec「禁用用户即刻失效」）。
        let (status, v) = request(
            &app,
            "POST",
            &format!("/api/users/{bob_id}/enabled"),
            Some(&alice),
            Some(r#"{"enabled":false}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let (status, _) = request(&app, "GET", "/api/summary", Some(&new_cookie), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "禁用必须踢会话");
        // 禁用后登录被拒（与凭据错误同文案 401）。
        let (status, _) = request(
            &app,
            "POST",
            "/api/auth/login",
            None,
            Some(r#"{"username":"bob","password":"new-pass-99"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // 未知用户 → 404。
        let (status, _) = request(
            &app,
            "POST",
            "/api/users/424242/enabled",
            Some(&alice),
            Some(r#"{"enabled":false}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn role_change_takes_effect_without_relogin() {
        let (app, _dir, auth) = rbac_app().await;
        let alice = login_cookie(&app, "alice", "password8").await;
        let bob = login_cookie(&app, "bob", "password8").await;
        let bob_id = auth
            .user_store()
            .unwrap()
            .get_by_username("bob")
            .unwrap()
            .unwrap()
            .id;

        // member 在线写会话 → 到达 handler（不被角色拦）。
        let (status, _) = request(
            &app,
            "POST",
            "/api/sessions",
            Some(&bob),
            Some(r#"{"agent":"native"}"#.into()),
        )
        .await;
        assert_ne!(status, StatusCode::FORBIDDEN, "member 会话写应放行");

        // 降级 viewer：同一会话下一个写操作 403（无需重登）。
        let (status, v) = request(
            &app,
            "POST",
            &format!("/api/users/{bob_id}/role"),
            Some(&alice),
            Some(r#"{"role":"viewer"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let (status, _) = request(
            &app,
            "POST",
            "/api/sessions",
            Some(&bob),
            Some(r#"{"agent":"native"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "角色调整必须即时生效");

        // 词表外角色 → 400。
        let (status, _) = request(
            &app,
            "POST",
            &format!("/api/users/{bob_id}/role"),
            Some(&alice),
            Some(r#"{"role":"boss"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn last_root_and_self_are_protected() {
        let (app, _dir, auth) = rbac_app().await;
        let alice = login_cookie(&app, "alice", "password8").await;
        let store = auth.user_store().unwrap();
        let alice_id = store.get_by_username("alice").unwrap().unwrap().id;
        let ada_id = store.get_by_username("ada").unwrap().unwrap().id;

        // 删号前先给 bob 换个会话（删号后旧 cookie 必须失效）。
        let bob_cookie = login_cookie(&app, "bob", "password8").await;
        let bob_id = store.get_by_username("bob").unwrap().unwrap().id;

        // 不能删自己 → 400。
        let (status, v) =
            request(&app, "DELETE", &format!("/api/users/{alice_id}"), Some(&alice), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");

        // 最后启用的 root：降级 / 禁用 / 删除全 400，root 原状。
        let (status, v) = request(
            &app,
            "POST",
            &format!("/api/users/{alice_id}/role"),
            Some(&alice),
            Some(r#"{"role":"member"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
        let (status, v) = request(
            &app,
            "POST",
            &format!("/api/users/{alice_id}/enabled"),
            Some(&alice),
            Some(r#"{"enabled":false}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
        let (status, v) =
            request(&app, "DELETE", &format!("/api/users/{alice_id}"), Some(&alice), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
        let (_, v) = request(&app, "GET", "/api/auth/me", Some(&alice), None).await;
        assert_eq!(v["role"], "root", "最后 root 必须保持原状: {v}");

        // 删除普通成员：200；其既有会话失效；重放登录被拒（用户没了）。
        let (status, v) =
            request(&app, "DELETE", &format!("/api/users/{bob_id}"), Some(&alice), None).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let (status, _) = request(&app, "GET", "/api/summary", Some(&bob_cookie), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "删号后旧会话必须失效");
        let (status, _) = request(
            &app,
            "POST",
            "/api/auth/login",
            None,
            Some(r#"{"username":"bob","password":"password8"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "已删用户登录必须拒绝");

        // ada（admin）不受最后 root 保护，可删；不存在的 id → 404。
        let (status, _) =
            request(&app, "DELETE", &format!("/api/users/{ada_id}"), Some(&alice), None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = request(&app, "DELETE", "/api/users/424242", Some(&alice), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
