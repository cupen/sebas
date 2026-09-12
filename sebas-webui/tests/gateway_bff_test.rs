//! Router BFF 集成测试（make-core-own-provider-data 3.1/3.2/3.3）：
//! provider 管理面改由 core 状态库承载——`RouterClient` 只剩 reload 代理，
//! BFF 处理器读写 `SessionBackend` 的 state seam。
//!
//! - 3.2：create → GET 立即读到新值（无重启）；update / delete / aliases
//!   / presets 走同一 seam。
//! - 3.3：core 不可达时 mutation 503、GET 不返回陈数据。
//! - 既有守卫不变：GET → 405；非 loopback origin → 403。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use sebas_dispatch::state_store::PersistedState;
use sebas_feishu::cards::CardConfig;
use sebas_webui::build_router;
use sebas_webui::models::RouterInfo;
use std::sync::{Arc, Mutex, Once, OnceLock};
use tower::ServiceExt;

// ---- 进程级内存引擎（provider 真源；ENGINE 是全局 OnceLock，只能 init 一次）----

#[derive(Default)]
struct MemoryInner {
    state: Mutex<PersistedState>,
}

struct MemoryEngine {
    inner: Arc<MemoryInner>,
}

fn memory_engine() -> Arc<MemoryInner> {
    static INNER: OnceLock<Arc<MemoryInner>> = OnceLock::new();
    static INIT: Once = Once::new();
    let inner = INNER.get_or_init(|| Arc::new(MemoryInner::default()));
    INIT.call_once(|| {
        sebas_dispatch::state_store::init_engine(Box::new(MemoryEngine {
            inner: inner.clone(),
        }));
    });
    inner.clone()
}

#[async_trait::async_trait]
impl sebas_dispatch::state_store::StateStoreEngine for MemoryEngine {
    async fn load_persisted_state(&self) -> PersistedState {
        self.inner.state.lock().unwrap().clone()
    }
    async fn save_persisted_state(&self, state: PersistedState) -> anyhow::Result<()> {
        *self.inner.state.lock().unwrap() = state;
        Ok(())
    }
    async fn load_settings(&self) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }
    async fn save_settings(&self, _cfg: serde_json::Value) -> Result<(), String> {
        Ok(())
    }
    async fn load_projects(&self) -> Result<Vec<serde_json::Value>, String> {
        Ok(Vec::new())
    }
    async fn save_projects(&self, _projects: Vec<serde_json::Value>) -> Result<(), String> {
        Ok(())
    }
    async fn add_project(&self, _path: &str, _name: &str, _added_at: i64) -> Result<(), String> {
        Ok(())
    }
    async fn remove_project(&self, _path: &str) -> Result<bool, String> {
        Ok(true)
    }
    async fn set_project_default_agent(&self, _id: &str, _agent: &str) -> Result<(), String> {
        Ok(())
    }
}

/// 带「core 状态库可达」后端的 app（InProcessBackend → 内存引擎）。
async fn app_with_core_store() -> (axum::Router, Arc<MemoryInner>) {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let backend: Arc<dyn sebas_webui::SessionBackend> =
        Arc::new(sebas_webui::session_backend::InProcessBackend::new(router));
    let app = build_router(backend, snapshot_router_info(), CardConfig::default());
    (app, memory_engine())
}

/// core 不可达形态：FakeBackend 默认 state_snapshot=None / state_mutate=Err
/// （真源离线；不得回退任何陈快照）。
async fn app_with_unreachable_core() -> axum::Router {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let backend: Arc<dyn sebas_webui::SessionBackend> = {
        let fake = sebas_webui::session_backend::FakeBackend::new();
        fake.set_reachable(false, "core session channel socket not found");
        Arc::new(fake)
    };
    let _ = router;
    build_router(backend, snapshot_router_info(), CardConfig::default())
}

fn snapshot_router_info() -> RouterInfo {
    RouterInfo {
        listen: Some("127.0.0.1:59999".into()), // 无 router 监听（reload 代理的降级路径）
        provider_count: 1,
        debug: false,
        has_auth: true,
        providers: vec![sebas_webui::models::ProviderInfo {
            name: "snapshot-provider".into(),
            base_url_anthropic: Some("https://snapshot.example".into()),
            base_url_openai_chat: None,
            base_url_openai_responses: None,
            preset: None,
        }],
    }
}

async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn json_request(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<String>,
) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("origin", "http://127.0.0.1:8080");
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let resp = app
        .clone()
        .oneshot(builder.body(Body::from(body.unwrap_or_default())).unwrap())
        .await
        .unwrap();
    (resp.status(), body_string(resp.into_body()).await)
}

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 内存引擎是进程级单例：触碰 store 的用例持锁串行 + 每次重置状态，
/// 避免并行用例互踩。
static STORE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn reset_store() -> tokio::sync::MutexGuard<'static, ()> {
    let g = STORE_LOCK.lock().await;
    *memory_engine().state.lock().unwrap() = PersistedState::default();
    g
}

#[tokio::test]
async fn api_router_reports_snapshot_when_router_down() {
    // SSR 网关页已由 SPA 取代；等价语义改为 API 面：/api/router 返回启动
    // 快照（providers 含 snapshot-provider），SPA 侧自行渲染降级提示。
    let (app, _mem) = app_with_core_store().await;
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
    assert!(
        body.contains("snapshot-provider"),
        "保底显示启动快照: {body}"
    );
}

/// 3.2 验收：创建 provider 后 GET **立即**读到新值（无重启）——读与写同一
/// core 状态库。
#[tokio::test]
async fn provider_create_is_immediately_visible_without_restart() {
    let _g = reset_store().await;
    let (app, _mem) = app_with_core_store().await;

    // 初始列表为空（store 是唯一事实来源，种子 provider 不出现）。
    let (status, body) = json_request(&app, "GET", "/router/api/providers", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["providers"].as_array().unwrap().len(), 0, "{body}");

    // 创建（preset deepseek；写经 providers 域落 core 状态库）。
    let (status, body) = json_request(
        &app,
        "POST",
        "/router/api/providers",
        Some(r#"{"name":"alpha","preset":"deepseek","api_key":"sk-a"}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // GET 立即读到新值（无重启），preset 槽位从代码表物化。
    let (status, body) = json_request(&app, "GET", "/router/api/providers", None).await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let rows = v["providers"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{body}");
    assert_eq!(rows[0]["name"], "alpha");
    assert_eq!(rows[0]["preset"], "deepseek");
    assert_eq!(
        rows[0]["base_url_anthropic"], "https://api.deepseek.com/anthropic",
        "preset 派生槽位由代码表物化: {body}"
    );
    assert_eq!(rows[0]["api_key_configured"], true, "明文 key → configured");
    assert!(!body.contains("sk-a"), "列表不得携带 key 材料: {body}");
}

/// 3.2：重名 409；更新空 api_key 保留旧值；删除后消失；删除未知 404。
#[tokio::test]
async fn provider_mutation_semantics_preserved_over_core_store() {
    let _g = reset_store().await;
    let (app, mem) = app_with_core_store().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/router/api/providers",
        Some(r#"{"name":"alpha","preset":"deepseek","api_key":"sk-keep"}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // 重名 → 409。
    let (status, body) = json_request(
        &app,
        "POST",
        "/router/api/providers",
        Some(r#"{"name":"alpha","preset":"deepseek"}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // 更新空 api_key → 保留旧值。
    let (status, body) = json_request(
        &app,
        "PUT",
        "/router/api/providers/alpha",
        Some(r#"{"name":"alpha","preset":"deepseek","api_key":""}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let key = mem.state.lock().unwrap().providers["alpha"]["api_key"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(key, "sk-keep", "空 key 提交须保留旧值");

    // 非法条目（未知字段）→ 400（core 校验，typed rejection）。
    let (status, body) = json_request(
        &app,
        "PUT",
        "/router/api/providers/alpha",
        Some(r#"{"name":"alpha","bogus_field":1}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // 删除 → 200 且列表回到空。
    let (status, _) = json_request(&app, "DELETE", "/router/api/providers/alpha", None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = json_request(&app, "GET", "/router/api/providers", None).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["providers"].as_array().unwrap().len(), 0, "{body}");

    // 删除未知 → 404。
    let (status, _) = json_request(&app, "DELETE", "/router/api/providers/ghost", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// 3.2：presets 读 seam（代码表直出）。
#[tokio::test]
async fn presets_served_from_code_table_via_backend() {
    let (app, _mem) = app_with_core_store().await;
    let (status, body) = json_request(&app, "GET", "/router/api/presets", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let presets = v["presets"].as_array().unwrap();
    assert!(
        presets.iter().any(|p| p["name"] == "deepseek"),
        "preset 表随代码: {body}"
    );
}

/// 3.3 验收：core 不可达时 mutation 返回 503，GET 不返回过期数据（503）。
#[tokio::test]
async fn unreachable_core_answers_503_without_stale_snapshot() {
    let app = app_with_unreachable_core().await;
    let _g = ENV_LOCK.lock().await;

    // GET：真源离线 → 503，绝不拿陈快照充数。
    let (status, body) = json_request(&app, "GET", "/router/api/providers", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    // POST mutation → 503。
    let (status, body) = json_request(
        &app,
        "POST",
        "/router/api/providers",
        Some(r#"{"name":"x","preset":"deepseek"}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    // DELETE mutation → 503。
    let (status, _) = json_request(&app, "DELETE", "/router/api/providers/x", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

/// probe 路由（add-fetch-models）：抓取由 core providers 域 op 承载——
/// 上游打向**本进程内的本地 mock**（绝不外联）。抓取 200 返回 id 列表、
/// 不改 store 任何字节、响应无密钥材料。
#[tokio::test]
async fn provider_probe_fetches_models_without_persisting() {
    let _g = reset_store().await;
    // 本地 mock /models：记录 Authorization，回固定 id 列表。
    let authz: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let authz_clone = authz.clone();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in listener.incoming().flatten() {
            let mut s = stream;
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let req = String::from_utf8_lossy(&buf).to_string();
            if let Some(line) = req
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("authorization"))
            {
                authz_clone.lock().unwrap().push(line.to_string());
            }
            let body = r#"{"object":"list","data":[{"id":"bff-m1"},{"id":"bff-m2"}]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });

    let (app, mem) = app_with_core_store().await;
    let mock = format!("http://{addr}/v1");

    // 普通编辑路径建一个自定义 provider（明文 key 只进 store）。
    let (status, body) = json_request(
        &app,
        "POST",
        "/router/api/providers",
        Some(
            serde_json::json!({
                "name": "mocko",
                "base_url_openai_chat": mock,
                "api_key": "sk-bff-secret-9"
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let before = serde_json::to_string(&mem.state.lock().unwrap().clone()).unwrap();

    let (status, body) =
        json_request(&app, "POST", "/router/api/providers/mocko/probe", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["provider"], "mocko");
    assert_eq!(
        v["models"],
        serde_json::json!(["bff-m1", "bff-m2"]),
        "fetch returns the upstream ids: {body}"
    );
    // 密钥材料绝不出现在响应。
    assert!(
        !body.contains("sk-bff-secret-9"),
        "no key in response: {body}"
    );
    // 上游请求带 Bearer key。
    let last = authz.lock().unwrap().last().cloned().unwrap_or_default();
    assert!(
        last.to_ascii_lowercase().contains("bearer sk-bff-secret-9"),
        "upstream call authenticated: {last}"
    );
    // 抓取不落盘：store 快照逐字节一致（选择模型是后续普通编辑）。
    let after = serde_json::to_string(&mem.state.lock().unwrap().clone()).unwrap();
    assert_eq!(before, after, "fetch must persist nothing");
}

/// probe：未知 provider → 404（typed rejection naming the reason）。
#[tokio::test]
async fn provider_probe_unknown_provider_answers_404() {
    let _g = reset_store().await;
    let (app, _mem) = app_with_core_store().await;
    let (status, body) =
        json_request(&app, "POST", "/router/api/providers/ghost/probe", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body.contains("不存在"), "{body}");
}

/// probe：无可用 base url → 400，且不发起上游请求。
#[tokio::test]
async fn provider_probe_without_base_url_answers_400() {
    let _g = reset_store().await;
    let (app, _mem) = app_with_core_store().await;
    let (status, body) = json_request(
        &app,
        "POST",
        "/router/api/providers",
        Some(r#"{"name":"urlless"}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, body) =
        json_request(&app, "POST", "/router/api/providers/urlless/probe", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("base URL"), "{body}");
}

/// probe：core 不可达 → 诚实 503（不伪造成功、不回退）。
#[tokio::test]
async fn provider_probe_unreachable_core_answers_503() {
    let app = app_with_unreachable_core().await;
    let _g = ENV_LOCK.lock().await;
    let (status, body) =
        json_request(&app, "POST", "/router/api/providers/alpha/probe", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
}

/// 既有守卫不变：mutation 子 router 的 POST-only（GET → 405）+ 非 loopback
/// origin → 403。
#[tokio::test]
async fn mutation_routes_guarded() {
    let (app, _mem) = app_with_core_store().await;
    for uri in ["/router/api/model-aliases", "/router/api/reload"] {
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED, "GET {uri}");
    }
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
}

/// workbench-conversation-view 4.1：`GET /router/api/defaults` 预选数据。
/// 数据面沿 c4r 契约：router 的 /admin/defaults 已下线，defaults 真源在
/// core 状态库（providers 域快照的 default_selection 段）。未设置 → 双
/// null；core 不可达 → 503。
#[tokio::test]
async fn router_api_defaults_reads_core_store_and_degrades_honestly() {
    let _g = ENV_LOCK.lock().await;
    let _reset = reset_store().await;
    let (app, mem) = app_with_core_store().await;

    // 未设置：双 null（语义照旧）。
    let (status, body) = json_request(&app, "GET", "/router/api/defaults", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["default_provider"].is_null(), "{v}");
    assert!(v["default_model"].is_null(), "{v}");

    // 设置 default_selection（写 providers 域快照同源）→ 正常读回。
    mem.state.lock().unwrap().default_selection = Some(
        sebas_dispatch::state_store::DefaultSelection::with_model("deepseek", "deepseek-chat"),
    );
    let (status, body) = json_request(&app, "GET", "/router/api/defaults", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["default_provider"], "deepseek", "{v}");
    assert_eq!(v["default_model"], "deepseek-chat", "{v}");

    // 只设 provider（model 缺省）→ model 为 null，provider 照读。
    mem.state.lock().unwrap().default_selection = Some(
        sebas_dispatch::state_store::DefaultSelection::new("anthropic"),
    );
    let (status, body) = json_request(&app, "GET", "/router/api/defaults", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["default_provider"], "anthropic", "{v}");
    assert!(v["default_model"].is_null(), "{v}");

    // core 不可达 → 503，绝不伪造默认值。
    let app = app_with_unreachable_core().await;
    let (status, body) = json_request(&app, "GET", "/router/api/defaults", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
}

// ---- unify-router-process-shape 2.3：admin adapter 的 force 透传与拒绝载荷 ----

/// 记录 service_set 实参的假 adapter：`reject_count` 非 None 时对未 force
/// 的停 router 回保护拒绝（与 watchdog executor 同语义）。
struct RecordingAdapter {
    reject_count: Option<u64>,
    seen_force: Mutex<Vec<bool>>,
}

impl RecordingAdapter {
    fn rejecting(count: u64) -> Arc<Self> {
        Arc::new(Self {
            reject_count: Some(count),
            seen_force: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl sebas_webui::admin::AdminAdapter for RecordingAdapter {
    async fn status(&self) -> Result<sebas_webui::admin::AdminStatus, String> {
        Err("not used".into())
    }
    async fn events_since(&self, _seq: u64) -> Result<Vec<sebas_webui::admin::AdminEvent>, String> {
        Err("not used".into())
    }
    async fn update(
        &self,
        _dev: bool,
        _dry_run: bool,
    ) -> Result<sebas_webui::admin::AdminMutationResult, String> {
        Err("not used".into())
    }
    async fn rollback(
        &self,
        _dry_run: bool,
    ) -> Result<sebas_webui::admin::AdminMutationResult, String> {
        Err("not used".into())
    }
    async fn restart_core(&self) -> Result<sebas_webui::admin::AdminMutationResult, String> {
        Err("not used".into())
    }
    async fn service_restart(
        &self,
        _service: &str,
    ) -> Result<sebas_webui::admin::AdminMutationResult, String> {
        Err("not used".into())
    }
    async fn services(&self) -> Result<Vec<sebas_webui::admin::AdminService>, String> {
        Err("not used".into())
    }
    async fn service_set(
        &self,
        service: &str,
        desired: &str,
        force: bool,
    ) -> Result<sebas_webui::admin::AdminMutationResult, sebas_webui::admin::AdminActionError>
    {
        self.seen_force.lock().unwrap().push(force);
        if let Some(count) = self.reject_count.filter(|_| !force) {
            return Err(sebas_webui::admin::AdminActionError {
                code: sebas_webui::admin::ACTIVE_ROUTED_SESSIONS_CODE.into(),
                message: format!("router 有 {count} 个活跃 routed 会话"),
                count: Some(count),
            });
        }
        Ok(sebas_webui::admin::AdminMutationResult {
            operation_id: format!("op_service_set_{service}"),
            status: "accepted".into(),
            message: format!("{service} set to {desired}"),
        })
    }
}

async fn admin_app_with(adapter: Arc<RecordingAdapter>) -> axum::Router {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let backend: Arc<dyn sebas_webui::SessionBackend> =
        Arc::new(sebas_webui::session_backend::InProcessBackend::new(router));
    let _ = memory_engine();
    sebas_webui::build_router_with_admin_adapter(
        backend,
        snapshot_router_info(),
        CardConfig::default(),
        Some(adapter as Arc<dyn sebas_webui::admin::AdminAdapter>),
    )
}

/// wire 合同：停 router 被拒 → HTTP 400，响应体顶层含
/// `code = "active_routed_sessions"` 与 `count = <数字>`。
#[tokio::test]
async fn admin_bff_router_stop_rejection_answers_400_with_code_and_count() {
    let adapter = RecordingAdapter::rejecting(4);
    let app = admin_app_with(adapter).await;
    let (status, body) = json_request(
        &app,
        "POST",
        "/api/admin/services/router/disable",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["code"], "active_routed_sessions", "{body}");
    assert_eq!(v["count"], 4, "{body}");
    assert!(v["error"].as_str().is_some(), "error 信封字段保留: {body}");
}

/// 强制出口流：force 重发（`{"force": true}`）透传 adapter 并放行 → 200。
#[tokio::test]
async fn admin_bff_router_stop_force_is_forwarded_and_accepted() {
    let adapter = RecordingAdapter::rejecting(2);
    let app = admin_app_with(adapter.clone()).await;

    // 无 force：被拒。
    let (status, body) = json_request(
        &app,
        "POST",
        "/api/admin/services/router/disable",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // force 重发：放行。
    let (status, body) = json_request(
        &app,
        "POST",
        "/api/admin/services/router/disable",
        Some(r#"{"force": true}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["status"], "accepted", "{body}");
    // force 实参按序透传（false → true）。
    assert_eq!(*adapter.seen_force.lock().unwrap(), vec![false, true]);
}
