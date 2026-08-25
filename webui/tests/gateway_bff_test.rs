//! Gateway BFF 集成测试（Task 6.2/6.3）：
//! - `/gateway` 动态拉取 admin API 数据（live 模式）+ gateway 关闭时降级。
//! - mutation 路由：GET → 405；非 loopback origin → 403；无 secret → 503；
//!   有 secret 时转发 admin API（create → 列表出现 → delete → 消失）。

use acp_claude::manager::SessionManager;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use router::router::RouterHandle;
use router::state::SessionMap;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use webui::models::GatewayInfo;
use webui::{build_router, init_templates_for_tests};

async fn app_with(gateway_listen: Option<String>) -> axum::Router {
    let (router, _rx) = RouterHandle::new(SessionMap::new());
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(5)));
    let templates = Arc::new(init_templates_for_tests());
    let gw = GatewayInfo {
        listen: gateway_listen,
        provider_count: 1,
        debug: false,
        has_auth: true,
        providers: vec![webui::models::ProviderInfo {
            name: "snapshot-provider".into(),
            base_url_anthropic: Some("https://snapshot.example".into()),
            base_url_openai: None,
        }],
    };
    build_router(router, mgr, gw, templates)
}

async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn gateway_page_degrades_when_gateway_down() {
    let app = app_with(Some("127.0.0.1:59999".into())).await; // 无 gateway 监听
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/gateway")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "降级页须 200");
    let html = body_string(resp.into_body()).await;
    assert!(html.contains("snapshot-provider"), "保底显示启动快照: 截断");
    assert!(html.contains("不可达") || html.contains("startup snapshot"), "降级提示");
}

#[tokio::test]
async fn gateway_page_live_when_gateway_up() {
    // 起 mock gateway admin 面。
    let admin = axum::Router::new()
        .route(
            "/admin/providers",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({"providers": [{"name": "live-alpha", "api_key_configured": true}]}))
            }),
        )
        .route(
            "/admin/model-aliases",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({"model_aliases": {"fast": {"provider": "live-alpha"}}}))
            }),
        )
        .route(
            "/admin/stats",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({"uptime_secs": 42, "per_provider": []}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, admin).await.unwrap();
    });

    let app = app_with(Some(format!("{addr}"))).await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/gateway")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let html = body_string(resp.into_body()).await;
    assert!(html.contains("live-alpha"), "页面须显示实时 provider");
    assert!(html.contains("fast"), "页面须显示别名");
    assert!(!html.contains("不可达"), "不应有降级提示");
}

/// SEBAS_CONTROL_SECRET 是进程级 env，涉及读写的测试必须串行。
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tokio::test]
async fn mutation_routes_guarded() {
    let _g = ENV_LOCK.lock().unwrap();
    let app = app_with(Some("127.0.0.1:59999".into())).await;
    // GET → 405（POST-only）。
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/gateway/api/providers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    // 非 loopback origin → 403。
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/gateway/api/providers")
                .header("origin", "http://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    // 无 secret → 503（loopback origin 放行到 handler 后拒绝）。
    unsafe { std::env::remove_var("SEBAS_CONTROL_SECRET") };
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/gateway/api/providers")
                .header("origin", "http://127.0.0.1:8080")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"x"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn mutation_round_trip_via_admin_api() {
    let _g = ENV_LOCK.lock().unwrap();
    // mock gateway admin：create → list 出现 → delete → 消失。
    use std::sync::atomic::{AtomicUsize, Ordering};
    let created = Arc::new(AtomicUsize::new(0));
    let deleted = Arc::new(AtomicUsize::new(0));
    let c2 = created.clone();
    let d2 = deleted.clone();
    let admin = axum::Router::new()
        .route(
            "/admin/providers",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({"providers": [{"name": "alpha"}]}))
            })
            .post(move |body: String| {
                let c = c2.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    let _ = body;
                    axum::Json(serde_json::json!({"created": "alpha"}))
                }
            }),
        )
        .route(
            "/admin/providers/{name}",
            axum::routing::delete(move |axum::extract::Path(name): axum::extract::Path<String>| {
                let d = d2.clone();
                async move {
                    d.fetch_add(1, Ordering::SeqCst);
                    axum::Json(serde_json::json!({"deleted": name}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, admin).await.unwrap();
    });

    unsafe { std::env::set_var("SEBAS_CONTROL_SECRET", "sec-1") };
    let app = app_with(Some(format!("{addr}"))).await;
    // create
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/gateway/api/providers")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"alpha","preset":"deepseek"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "{}", body_string(resp.into_body()).await);
    // delete
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/gateway/api/providers/alpha")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(created.load(Ordering::SeqCst), 1);
    assert_eq!(deleted.load(Ordering::SeqCst), 1);
    unsafe { std::env::remove_var("SEBAS_CONTROL_SECRET") };
}
