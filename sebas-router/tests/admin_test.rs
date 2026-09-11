//! Admin API 集成测试（router-admin-api；make-core-own-provider-data 5.1）。
//!
//! 覆盖：鉴权（bearer / loopback fallback / 401 不回显）、admin 路由不被
//! proxy fallback 吞、provider/alias/defaults/probe 变更面下线（全部 404
//! 且不写任何文件，2.1/2.2）、只读面健在（presets / reload / stats /
//! metrics / 外部热重载，2.4）。

mod support;

use std::time::Duration;

use serde_json::{json, Value};
use support::start_router;

/// 测试 config：provider 全走 preset，overlay 指向 tempdir（由调用方通过
/// env 注入路径——见 start_router_admin）。
const CFG_TMPL: &str = r#"
[router]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"

[provider.anthropic]
api_key_env = "SEBAS_ROUTER_TEST_UPSTREAM_KEY"

[provider.openai]
api_key_env = "SEBAS_ROUTER_TEST_UPSTREAM_KEY_OAI"
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
        std::env::set_var("SEBAS_ROUTER_PROVIDER_OVERLAY", overlay.to_str().unwrap());
        std::env::set_var("SEBAS_ROUTER_CONFIG", cfg_path.to_str().unwrap());
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
            "SEBAS_ROUTER_PROVIDER_OVERLAY",
            overlay.to_str().unwrap(),
        );
        std::env::set_var(
            "SEBAS_ROUTER_CONFIG",
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

/// 带独立 overlay 的 router 启动。env（overlay 路径 + secret）必须在
/// start_router **之前**注入——config 在启动时解析 overlay；否则会读到
/// 开发机真实的 ~/.sebas/providers.json。
async fn start_admin_gw(secret: Option<&str>) -> (
    support::TestRouter,
    std::path::PathBuf,
    EnvGuard,
) {
    let dir = tempfile_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let overlay = dir.join("providers.json");
    // reload_and_swap 从 config_source 重读 toml 种子——写一份真实文件并经
    // SEBAS_ROUTER_CONFIG 注入，避免读到开发机的 ~/.sebas/config.toml。
    let cfg_path = dir.join("config.toml");
    std::fs::write(&cfg_path, CFG_TMPL.replace("__USAGE__", "")).unwrap();
    let env = set_envs(&overlay, secret, &cfg_path);
    let gw = start_router(CFG_TMPL).await;
    (gw, overlay, env)
}

/// tempdir helper（tempfile crate 在 router dev-deps 里）。用原子计数保证
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
        .get(format!("http://{}/admin/presets", gw.addr))
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
        .get(format!("http://{}/admin/presets", gw.addr))
        .header("Authorization", "Bearer sec-test-123")
        .send()
        .await
        .expect("GET");
    assert_eq!(resp.status(), 200, "admin 路由须答 200 而非 proxy 404");
    let body: Value = serde_json::from_str(&resp.text().await.expect("body")).expect("json");
    assert!(body["presets"].is_array(), "body: {body}");
}

#[tokio::test]
async fn admin_loopback_ok_without_secret() {
    let (gw, _overlay, _env) = start_admin_gw(None).await;
    // 测试 client 从 loopback 发起 → 无 secret 也放行。
    let resp = client()
        .get(format!("http://{}/admin/presets", gw.addr))
        .send()
        .await
        .expect("GET");
    assert_eq!(resp.status(), 200, "loopback + 无 secret 须放行");
}

/// provider/alias/defaults/probe 变更面整体下线（make-core-own-provider-data
/// 2.1）：任何方法（含 GET——「Provider CRUD endpoints」需求整体移除）都答
/// 404（不是 503 桩，也不得落回 proxy fallback 被透传上游）。
#[tokio::test]
async fn retired_provider_surface_answers_404() {
    let (gw, _overlay, _env) = start_admin_gw(Some("sec-test-123")).await;
    let base = format!("http://{}/admin", gw.addr);
    let auth = |r: reqwest::RequestBuilder| r.header("Authorization", "Bearer sec-test-123");
    let c = client();

    for (method, url) in [
        ("GET", "/providers"),
        ("POST", "/providers"),
        ("PUT", "/providers/deepseek"),
        ("DELETE", "/providers/deepseek"),
        ("POST", "/providers/deepseek/probe"),
        ("POST", "/providers/deepseek/probe?apply=true"),
        ("GET", "/model-aliases"),
        ("POST", "/model-aliases"),
        ("PUT", "/model-aliases/fast"),
        ("DELETE", "/model-aliases/fast"),
        ("GET", "/defaults"),
        ("PUT", "/defaults"),
    ] {
        let resp = auth(match method {
            "GET" => c.get(format!("{base}{url}")),
            "POST" => c
                .post(format!("{base}{url}"))
                .header("content-type", "application/json")
                .body(json!({"name": "x"}).to_string()),
            "PUT" => c
                .put(format!("{base}{url}"))
                .header("content-type", "application/json")
                .body(json!({"name": "x"}).to_string()),
            "DELETE" => c.delete(format!("{base}{url}")),
            other => panic!("unexpected method {other}"),
        })
        .send()
        .await
        .unwrap();
        assert_eq!(resp.status(), 404, "{method} {url} must be gone");
        let body: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
        assert!(
            body["error"].as_str().is_some_and(|e| e.contains("retired")),
            "404 body must state retirement: {body}"
        );
    }
}

/// 2.2 验收：无 core 通道（SEBAS_CORE_SOCKET 未设置）时，任何管理操作都
/// 不写文件——providers.json / defaults.json 从未被创建或改动。
#[tokio::test]
async fn admin_mutations_write_no_files_without_core_channel() {
    let (gw, overlay, _env) = start_admin_gw(Some("sec-test-123")).await;
    let base = format!("http://{}/admin", gw.addr);
    let auth = |r: reqwest::RequestBuilder| r.header("Authorization", "Bearer sec-test-123");
    let c = client();

    let before = std::fs::read_to_string(&overlay).unwrap_or_default();
    for req in [
        c.post(format!("{base}/providers"))
            .header("content-type", "application/json")
            .body(json!({"name": "deepseek", "preset": "deepseek", "api_key": "sk-ds"}).to_string()),
        c.put(format!("{base}/providers/openai"))
            .header("content-type", "application/json")
            .body(json!({"api_key": "sk-new"}).to_string()),
        c.delete(format!("{base}/providers/openai")),
        c.post(format!("{base}/providers/openai/probe?apply=true")),
        c.post(format!("{base}/model-aliases"))
            .header("content-type", "application/json")
            .body(json!({"alias": "fast", "provider": "anthropic"}).to_string()),
        c.delete(format!("{base}/model-aliases/fast")),
        c.put(format!("{base}/defaults"))
            .header("content-type", "application/json")
            .body(json!({"provider": "anthropic"}).to_string()),
    ] {
        let resp = auth(req).send().await.unwrap();
        assert_eq!(resp.status(), 404, "retired route must 404");
    }
    let after = std::fs::read_to_string(&overlay).unwrap_or_default();
    assert_eq!(before, after, "overlay 文件不得被管理操作改动");
    let defaults = overlay.with_file_name("defaults.json");
    assert!(
        !defaults.exists(),
        "router 侧不得再有 defaults.json 写入点（2.3）"
    );
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

/// probe 端点下线（make-core-own-provider-data：写回路径归 core；上游抓取
/// 能力由 add-fetch-models 在 core 侧重做）→ 任何 probe 请求 404，且不发
/// 起任何上游请求。
#[tokio::test]
async fn probe_endpoint_is_gone() {
    let (gw, _overlay, _env) = start_admin_gw(Some("sec-test-123")).await;
    let c = client();
    for url in [
        format!("http://{}/admin/providers/anthropic/probe", gw.addr),
        format!("http://{}/admin/providers/anthropic/probe?apply=true", gw.addr),
    ] {
        let resp = c
            .post(&url)
            .header("Authorization", "Bearer sec-test-123")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404, "probe must be gone: {url}");
    }
}

#[tokio::test]
async fn hot_reload_external_write_and_failure_recovery() {
    use sebas_router::server;

    let dir = tempfile_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let overlay = dir.join("providers.json");
    let cfg_path = dir.join("config.toml");
    // 种子只有 anthropic；外部写（模拟卡片/router 写）加 deepseek。
    let cfg_toml = r#"
[router]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"

[provider.anthropic]
api_key_env = "SEBAS_ROUTER_TEST_UPSTREAM_KEY"
"#;
    let cfg_toml = cfg_toml.replace("__USAGE__", &dir.join("usage.jsonl").to_string_lossy().replace('\\', "/"));
    std::fs::write(&cfg_path, &cfg_toml).unwrap();
    let _env = set_envs_locked(&overlay, Some("sec-test-123"), &cfg_path);

    // 与 support::start_router 同款 env key 注入（本测试绕过其 harness）。
    unsafe {
        std::env::set_var("SEBAS_ROUTER_TEST_UPSTREAM_KEY", "test-anthropic-key");
    }
    let cfg = sebas_router::config::RouterConfig::parse(&cfg_toml).unwrap();
    let state = server::build_state(cfg).unwrap();
    let ready = sebas_router::hot_reload::spawn_watcher(state.clone(), state.reload_status.clone());
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
    use sebas_router::proto::WireProtocol;
    use support::{start_router, start_mock_upstream};

    // mock anthropic 上游 + 指向它的 provider，走真实 proxy 路径产流量。
    let mock = start_mock_upstream(WireProtocol::Anthropic).await;
    let cfg = format!(
        r#"
[router]
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
    let gw = start_router(&cfg).await;

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
    assert!(text.contains("# TYPE router_requests_total counter"), "HELP/TYPE 行: {text}");
    assert!(
        text.contains("router_requests_total{provider=\"alpha\""),
        "alpha series: {text}"
    );
    // start_time gauge（router-metrics spec）。
    assert!(text.contains("# TYPE router_start_time_seconds gauge"), "start_time TYPE: {text}");
    assert!(text.contains("router_start_time_seconds "), "start_time series: {text}");
    // 无 bearer 非 loopback 判定不适用于本测试 client（loopback）——鉴权路径
    // 已由其它测试覆盖；这里验证文本合法性（每行 name value）。
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        assert!(line.contains(' '), "series 行格式: {line}");
    }

    // /admin/stats：alpha 聚合 requests=3 + 全局 totals + 平均延迟。
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
    let totals = &body["totals"];
    assert_eq!(totals["requests"], 3, "全局 totals.requests: {body}");
    assert!(totals["input_tokens"].is_number() && totals["cache_tokens"].is_number(), "tokens totals: {body}");
    assert_eq!(totals["rate_limited"], 0, "rate_limited totals: {body}");
    assert_eq!(totals["upstream_errors"], 0, "upstream_errors totals: {body}");
    assert!(alpha["avg_latency_ms"].is_number(), "avg_latency_ms: {alpha}");
}

// -------------------- agent defaults（make-core-own-provider-data 2.3）--------------------

/// defaults 面整体由 core 承担：router 的 /admin/defaults 读写都下线（404），
/// 不再产生 defaults.json 写入点。
#[tokio::test]
async fn agent_defaults_surface_is_gone() {
    let (gw, overlay, _env) = start_admin_gw(Some("sec-test-123")).await;
    let url = format!("http://{}/admin/defaults", gw.addr);
    let auth = |r: reqwest::RequestBuilder| r.header("Authorization", "Bearer sec-test-123");

    let resp = auth(client().get(&url)).send().await.unwrap();
    assert_eq!(resp.status(), 404, "GET defaults must be gone");
    let resp = auth(
        client()
            .put(&url)
            .header("content-type", "application/json")
            .body(json!({"provider": "anthropic", "model": "claude-opus-4-20250514"}).to_string()),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), 404, "PUT defaults must be gone");
    assert!(
        !overlay.with_file_name("defaults.json").exists(),
        "router 侧不再产生 defaults.json"
    );
}
