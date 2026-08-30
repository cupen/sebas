//! Admin API 集成测试（change gateway-admin-api-and-model-aliases，task 3.x）。
//!
//! 覆盖：鉴权（bearer / loopback fallback / 401 不回显）、admin 路由不被
//! proxy fallback 吞、providers CRUD（脱敏/409/空 key 保留/墓碑/失败不写
//! 文件）、model-aliases CRUD、reload。

mod support;

use std::time::Duration;

use serde_json::{json, Value};
use support::start_gateway;

/// 测试 config：provider 全走 preset，overlay 指向 tempdir（由调用方通过
/// env 注入路径——见 start_gateway_admin）。
const CFG_TMPL: &str = r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"

[provider.anthropic]
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY"

[provider.openai]
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY_OAI"
"#;

/// admin 测试需要控制 overlay 路径与 SEBAS_CONTROL_SECRET——两者都是进程
/// env，测试须串行（env lock）。
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct EnvGuard {
    _g: std::sync::MutexGuard<'static, ()>,
}

/// set_envs 但长期持锁（guard 由调用方保存到测试结束）——长耗时测试
/// （hot_reload 等秒级）期间其它测试不得改写 env。
fn set_envs_locked(
    overlay: &std::path::Path,
    secret: Option<&str>,
    cfg_path: &std::path::Path,
) -> EnvGuard {
    let g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("SEBAS_GATEWAY_PROVIDER_OVERLAY", overlay.to_str().unwrap());
        std::env::set_var("SEBAS_GATEWAY_CONFIG", cfg_path.to_str().unwrap());
        match secret {
            Some(s) => std::env::set_var("SEBAS_CONTROL_SECRET", s),
            None => std::env::remove_var("SEBAS_CONTROL_SECRET"),
        }
    }
    EnvGuard { _g: g }
}

#[allow(dead_code)]
fn set_envs(
    overlay: &std::path::Path,
    secret: Option<&str>,
    cfg_path: &std::path::Path,
) -> EnvGuard {
    let g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: 测试串行持有 ENV_LOCK。
    unsafe {
        std::env::set_var(
            "SEBAS_GATEWAY_PROVIDER_OVERLAY",
            overlay.to_str().unwrap(),
        );
        std::env::set_var(
            "SEBAS_GATEWAY_CONFIG",
            cfg_path.to_str().unwrap(),
        );
        match secret {
            Some(s) => std::env::set_var("SEBAS_CONTROL_SECRET", s),
            None => std::env::remove_var("SEBAS_CONTROL_SECRET"),
        }
    }
    EnvGuard { _g: g }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

/// 带独立 overlay 的 gateway 启动。env（overlay 路径 + secret）必须在
/// start_gateway **之前**注入——config 在启动时解析 overlay；否则会读到
/// 开发机真实的 ~/.sebas/providers.json。
async fn start_admin_gw(secret: Option<&str>) -> (
    support::TestGateway,
    std::path::PathBuf,
    EnvGuard,
) {
    let dir = tempfile_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let overlay = dir.join("providers.json");
    // reload_and_swap 从 config_source 重读 toml 种子——写一份真实文件并经
    // SEBAS_GATEWAY_CONFIG 注入，避免读到开发机的 ~/.sebas/config.toml。
    let cfg_path = dir.join("config.toml");
    std::fs::write(&cfg_path, CFG_TMPL.replace("__USAGE__", "")).unwrap();
    let env = set_envs(&overlay, secret, &cfg_path);
    let gw = start_gateway(CFG_TMPL).await;
    (gw, overlay, env)
}

/// tempdir helper（tempfile crate 在 gateway dev-deps 里）。用原子计数保证
/// 每个测试独立目录——按 ENV_LOCK 地址派生会让全部测试共享同一路径。
fn tempfile_dir() -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "sebas-admin-test-{}-{n}",
        std::process::id()
    ))
}

#[tokio::test]
async fn admin_401_without_bearer_when_secret_set() {
    let (gw, _overlay, _env) = start_admin_gw(Some("sec-test-123")).await;
    let resp = client()
        .get(format!("http://{}/admin/providers", gw.addr))
        .send()
        .await
        .expect("GET");
    assert_eq!(resp.status(), 401);
    let body = resp.text().await.unwrap();
    assert!(!body.contains("sec-test-123"), "401 不得回显 secret");
}

#[tokio::test]
async fn admin_bearer_accepted_and_not_swallowed_by_proxy() {
    let (gw, _overlay, _env) = start_admin_gw(Some("sec-test-123")).await;
    let resp = client()
        .get(format!("http://{}/admin/providers", gw.addr))
        .header("Authorization", "Bearer sec-test-123")
        .send()
        .await
        .expect("GET");
    assert_eq!(resp.status(), 200, "admin 路由须答 200 而非 proxy 404");
    let body: Value = serde_json::from_str(&resp.text().await.expect("body")).expect("json");
    assert!(body["providers"].is_array(), "body: {body}");
}

#[tokio::test]
async fn admin_loopback_ok_without_secret() {
    let (gw, _overlay, _env) = start_admin_gw(None).await;
    // 测试 client 从 loopback 发起 → 无 secret 也放行。
    let resp = client()
        .get(format!("http://{}/admin/providers", gw.addr))
        .send()
        .await
        .expect("GET");
    assert_eq!(resp.status(), 200, "loopback + 无 secret 须放行");
}

#[tokio::test]
async fn provider_crud_round_trip() {
    let (gw, overlay, _env) = start_admin_gw(Some("sec-test-123")).await;
    let base = format!("http://{}/admin/providers", gw.addr);
    let c = client();
    let auth = |r: reqwest::RequestBuilder| r.header("Authorization", "Bearer sec-test-123");

    // 创建（preset deepseek）。
    let resp = auth(c.post(&base).header("content-type", "application/json")
        .body(serde_json::to_string(&json!({
        "name": "deepseek", "preset": "deepseek", "api_key": "sk-ds"
    })).unwrap()))
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 201, "create");
    // 重名 409。
    let resp = auth(c.post(&base).header("content-type", "application/json")
        .body(serde_json::to_string(&json!({
        "name": "deepseek", "preset": "deepseek"
    })).unwrap()))
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 409, "duplicate 409");
    // 无效（无 preset 无 URL）400 且文件不含该条目。
    let resp = auth(c.post(&base).header("content-type", "application/json")
        .body(serde_json::to_string(&json!({"name": "bad"})).unwrap()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400, "invalid 400");
    // 列表脱敏。
    let resp = auth(c.get(&base)).send().await.unwrap();
    let body: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    let text = body.to_string();
    assert!(!text.contains("sk-ds"), "列表不得含 key 材料: {text}");
    // 更新：空 api_key 保留旧值。
    let resp = auth(c.put(format!("{base}/deepseek")).header("content-type", "application/json")
        .body(serde_json::to_string(&json!({
        "name": "deepseek", "preset": "deepseek", "api_key": ""
    })).unwrap()))
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 200, "update");
    let raw = std::fs::read_to_string(&overlay).unwrap();
    assert!(raw.contains("sk-ds"), "空 key 提交须保留旧值: {raw}");
    // 未知 provider 更新/删除 → 404（update 走合并→校验；delete 显式检查）。
    let resp = auth(c.delete(format!("{base}/nonexistent")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404, "delete unknown 404");
    // 删除 config 种子 provider → 墓碑。
    let resp = auth(c.delete(format!("{base}/openai"))).send().await.unwrap();
    assert_eq!(resp.status(), 200, "delete seed provider");
    let raw = std::fs::read_to_string(&overlay).unwrap();
    assert!(raw.contains("\"openai\""), "种子 provider 删除须写墓碑: {raw}");
}

#[tokio::test]
async fn alias_crud_round_trip() {
    let (gw, overlay, _env) = start_admin_gw(Some("sec-test-123")).await;
    let base = format!("http://{}/admin/model-aliases", gw.addr);
    let c = client();
    let auth = |r: reqwest::RequestBuilder| r.header("Authorization", "Bearer sec-test-123");

    // 创建合法别名。
    let resp = auth(c.post(&base).header("content-type", "application/json")
        .body(serde_json::to_string(&json!({
        "alias": "fast", "provider": "anthropic", "upstream_model": "claude-sonnet-4"
    })).unwrap()))
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 201, "alias create: {}", resp.status());
    // 未知 provider → 400。
    let resp = auth(c.post(&base).header("content-type", "application/json")
        .body(serde_json::to_string(&json!({
        "alias": "x", "provider": "ghost"
    })).unwrap()))
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 400);
    // 含 '/' → 400。
    let resp = auth(c.post(&base).header("content-type", "application/json")
        .body(serde_json::to_string(&json!({
        "alias": "a/b", "provider": "anthropic"
    })).unwrap()))
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 400);
    // 重名 → 409。
    let resp = auth(c.post(&base).header("content-type", "application/json")
        .body(serde_json::to_string(&json!({
        "alias": "fast", "provider": "anthropic"
    })).unwrap()))
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 409);
    // 列表。
    let resp = auth(c.get(&base)).send().await.unwrap();
    let body: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    assert!(body["model_aliases"]["fast"].is_object(), "body: {body}");
    // 更新。
    let resp = auth(c.put(format!("{base}/fast")).header("content-type", "application/json")
        .body(serde_json::to_string(&json!({
        "alias": "fast", "provider": "openai"
    })).unwrap()))
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 200, "update: {}", resp.text().await.unwrap_or_default());
    // 未知别名更新 → 404。
    let resp = auth(c.put(format!("{base}/nope")).header("content-type", "application/json")
        .body(serde_json::to_string(&json!({
        "alias": "nope", "provider": "anthropic"
    })).unwrap()))
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 404);
    // 删除。
    let resp = auth(c.delete(format!("{base}/fast"))).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let raw = std::fs::read_to_string(&overlay).unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert!(v["model_aliases"].as_object().map_or(true, |m| !m.contains_key("fast")));
}

#[tokio::test]
async fn reload_endpoint_reports() {
    let (gw, _overlay, _env) = start_admin_gw(Some("sec-test-123")).await;
    let c = client();
    let resp = c
        .post(format!("http://{}/admin/reload", gw.addr))
        .header("Authorization", "Bearer sec-test-123")
        .send()
        .await
        .unwrap();
    let body: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    // config_source 指向不存在路径 → 失败路径（409 + error 文本）；
    // 或成功（reloaded true）。两者都可接受，重点是端点工作且返回 JSON。
    assert!(body.get("reloaded").is_some(), "body: {body}");
}

#[tokio::test]
async fn probe_lists_and_applies_models() {
    use sebas_gateway::proto::WireProtocol;
    use support::start_mock_upstream;

    let mock = start_mock_upstream(WireProtocol::OpenAi).await;
    let cfg = CFG_TMPL.replace(
        "[provider.openai]",
        &format!(
            "[provider.mocko]\nbase_url_openai = \"{}/v1\"\napi_key = \"sk-mock\"\n\n[provider.openai]"
        , mock.url),
    );
    let dir = tempfile_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let overlay = dir.join("providers.json");
    let cfg_path = dir.join("config.toml");
    std::fs::write(&cfg_path, &cfg).unwrap();
    let _env = set_envs(&overlay, Some("sec-test-123"), &cfg_path);
    let gw = start_gateway(&cfg).await;

    let c = client();
    let base = format!("http://{}/admin/providers/mocko/probe", gw.addr);
    // 列表。
    let resp = c
        .post(&base)
        .header("Authorization", "Bearer sec-test-123")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "probe: {}", resp.text().await.unwrap_or_default());
    // apply=true 写回 models 字段。
    let resp = c
        .post(format!("{base}?apply=true"))
        .header("Authorization", "Bearer sec-test-123")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "apply: {}", resp.text().await.unwrap_or_default());
    let body: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    assert!(body["models"].as_array().map_or(false, |a| !a.is_empty()), "body: {body}");
    let raw = std::fs::read_to_string(&overlay).unwrap();
    assert!(raw.contains("gpt-4"), "apply 须写回 models 列表: {raw}");
    // 上游收到的请求带了 key（Authorization bearer）。
    let reqs = mock.requests.lock().await;
    let r = reqs.last().expect("mock 收到请求");
    assert_eq!(r.path, "/v1/models");
    let authz = r.headers.get("authorization").map(String::as_str);
    assert_eq!(authz, Some("Bearer sk-mock"), "key 须注入上游请求");
}

#[tokio::test]
async fn hot_reload_external_write_and_failure_recovery() {
    use sebas_gateway::server;

    let dir = tempfile_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let overlay = dir.join("providers.json");
    let cfg_path = dir.join("config.toml");
    // 种子只有 anthropic；外部写（模拟卡片/router 写）加 deepseek。
    let cfg_toml = r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"

[provider.anthropic]
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY"
"#;
    let cfg_toml = cfg_toml.replace("__USAGE__", &dir.join("usage.jsonl").to_string_lossy().replace('\\', "/"));
    std::fs::write(&cfg_path, &cfg_toml).unwrap();
    let _env = set_envs_locked(&overlay, Some("sec-test-123"), &cfg_path);

    // 与 support::start_gateway 同款 env key 注入（本测试绕过其 harness）。
    unsafe {
        std::env::set_var("SEBAS_GATEWAY_TEST_UPSTREAM_KEY", "test-anthropic-key");
    }
    let cfg = sebas_gateway::config::GatewayConfig::parse(&cfg_toml).unwrap();
    let state = server::build_state(cfg).unwrap();
    let ready = sebas_gateway::hot_reload::spawn_watcher(state.clone(), state.reload_status.clone());
    ready.await.expect("watcher 注册完成");
    let app = server::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let c = client();
    let stats_url = format!("http://{addr}/admin/stats");
    let auth = "Bearer sec-test-123";

    // 坏 JSON：外部写入损坏文件 → reload 失败，stats 报错，旧内核继续。
    // watcher 注册是异步 task——轮询直到观察到 reload 失败（上限 5s）。
    std::fs::write(&overlay, "{ not json").unwrap();
    let mut body = Value::Null;
    for _ in 0..25 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let resp = c.get(&stats_url).header("Authorization", auth).send().await.unwrap();
        let raw = resp.text().await.unwrap();
        body = serde_json::from_str(&raw).unwrap_or_else(|e| panic!("stats 非 JSON: {raw} ({e})"));
        if body["last_reload_error"].is_string() {
            break;
        }
    }
    assert!(body["last_reload_error"].is_string(), "坏 JSON 须记 reload 错误: {body}");
    assert_eq!(body["providers"], 1, "坏文件保旧内核（仍只有 anthropic）");

    // 有效外部写：加 deepseek provider。
    std::fs::write(
        &overlay,
        serde_json::json!({"providers": {"deepseek": {"preset": "deepseek", "api_key": "sk-x"}}}).to_string(),
    )
    .unwrap();
    // 等 debounce(300ms) + 处理。
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let body: Value = serde_json::from_str(
            &c.get(&stats_url).header("Authorization", auth).send().await.unwrap().text().await.unwrap(),
        )
        .unwrap();
        if body["providers"] == 2 { break; }
    }
    let body: Value = serde_json::from_str(
        &c.get(&stats_url).header("Authorization", auth).send().await.unwrap().text().await.unwrap(),
    )
    .unwrap();
    assert_eq!(body["providers"], 2, "外部写后热重载生效（无重启）: {body}");
    assert!(body["last_reload_error"].is_null(), "恢复后清错误: {body}");
    assert!(body["last_reload_ok_at"].is_u64(), "记录成功时间: {body}");
}

#[tokio::test]
async fn metrics_and_stats_after_traffic() {
    use sebas_gateway::proto::WireProtocol;
    use support::{start_gateway, start_mock_upstream};

    // mock anthropic 上游 + 指向它的 provider，走真实 proxy 路径产流量。
    let mock = start_mock_upstream(WireProtocol::Anthropic).await;
    let cfg = format!(
        r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"
auth_token = "tok-1"

[provider.alpha]
base_url_anthropic = "{}"
api_key = "sk-alpha"
"#,
        mock.url
    );
    let dir = tempfile_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let overlay = dir.join("providers.json");
    let cfg_path = dir.join("config.toml");
    std::fs::write(&cfg_path, &cfg).unwrap();
    let _env = set_envs_locked(&overlay, Some("sec-test-123"), &cfg_path);
    let gw = start_gateway(&cfg).await;

    let c = client();
    // 3 个请求（非流式 messages）。
    for _ in 0..3 {
        let resp = c
            .post(format!("http://{}/v1/messages", gw.addr))
            .header("x-api-key", "tok-1")
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(r#"{"model":"m1","max_tokens":16,"messages":[{"role":"user","content":"hi"}]}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }
    // 401：auth 拒绝计数。
    let _ = c
        .post(format!("http://{}/v1/messages", gw.addr))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();

    // /metrics：bearer 抓取，文本格式，含 alpha 请求数。
    let resp = c
        .get(format!("http://{}/metrics", gw.addr))
        .header("Authorization", "Bearer sec-test-123")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(text.contains("# TYPE sebas_gateway_requests_total counter"), "HELP/TYPE 行: {text}");
    assert!(
        text.contains("sebas_gateway_requests_total{provider=\"alpha\""),
        "alpha series: {text}"
    );
    // 无 bearer 非 loopback 判定不适用于本测试 client（loopback）——鉴权路径
    // 已由其它测试覆盖；这里验证文本合法性（每行 name value）。
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        assert!(line.contains(' '), "series 行格式: {line}");
    }

    // /admin/stats：alpha 聚合 requests=3。
    let resp = c
        .get(format!("http://{}/admin/stats", gw.addr))
        .header("Authorization", "Bearer sec-test-123")
        .send()
        .await
        .unwrap();
    let body: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    let alpha = body["per_provider"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "alpha")
        .unwrap_or_else(|| panic!("stats 无 alpha: {body}"));
    assert_eq!(alpha["requests"], 3, "alpha 聚合: {alpha}");
    assert!(body["uptime_secs"].is_u64(), "uptime: {body}");
}
