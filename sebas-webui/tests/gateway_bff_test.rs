//! Router BFF 集成测试（Task 6.2/6.3）：
//! - `/router` 动态拉取 admin API 数据（live 模式）+ router 关闭时降级。
//! - mutation 路由：GET → 405；非 loopback origin → 403；无 secret → 503；
//!   有 secret 时转发 admin API（create → 列表出现 → delete → 消失）。

use sebas_feishu::cards::CardConfig;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use std::sync::Arc;
use tower::ServiceExt;
use sebas_webui::models::RouterInfo;
use sebas_webui::build_router;

async fn app_with(router_listen: Option<String>) -> axum::Router {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let backend: Arc<dyn sebas_webui::SessionBackend> =
        Arc::new(sebas_webui::session_backend::InProcessBackend::new(router));
    let gw = RouterInfo {
        listen: router_listen,
        provider_count: 1,
        debug: false,
        has_auth: true,
        providers: vec![sebas_webui::models::ProviderInfo {
            name: "snapshot-provider".into(),
            base_url_anthropic: Some("https://snapshot.example".into()),
            base_url_openai: None,
        }],
    };
    build_router(backend, gw, CardConfig::default())
}

async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn api_router_reports_snapshot_when_router_down() {
    // SSR 网关页已由 SPA 取代；等价语义改为 API 面：/api/router 返回启动
    // 快照（providers 含 snapshot-provider），SPA 侧自行渲染降级提示。
    let app = app_with(Some("127.0.0.1:59999".into())).await; // 无 router 监听
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/router")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "快照 API 须 200");
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("snapshot-provider"), "保底显示启动快照: {body}");
}

#[tokio::test]
async fn router_bff_read_routes_are_mutation_only() {
    // BFF mutation 面（POST/PUT/DELETE）不接受 GET——SPA 用 /api/router
    // 快照 + router 自身 admin API（经 BFF 转发）组合出只读视图。
    let app = app_with(Some("127.0.0.1:59999".into())).await;
    for uri in [
        "/router/api/providers",
        "/router/api/model-aliases",
        "/router/api/reload",
    ] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED, "GET {uri}");
    }
}

#[tokio::test]
async fn mutation_routes_guarded() {
    let _g = ENV_LOCK.lock().await;
    let app = app_with(Some("127.0.0.1:59999".into())).await;
    // GET → 405（POST-only）。
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/router/api/providers")
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
                .uri("/router/api/providers")
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
                .uri("/router/api/providers")
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
    let _g = ENV_LOCK.lock().await;
    // mock router admin：create → list 出现 → delete → 消失。
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
                .uri("/router/api/providers")
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
                .uri("/router/api/providers/alpha")
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
