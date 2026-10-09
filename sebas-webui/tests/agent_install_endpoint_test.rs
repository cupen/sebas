//! add-agent-auto-install 集成层：`POST /api/agents/install` 的**端点级**端到端
//! 路径——真实 axum handler + 真实状态库引擎（`InProcessBackend` 读全局
//! `StateStoreEngine`）+ PATH 垫 fake npm（design D7：绝不真拨 registry）。
//!
//! 与 `src/agent_install.rs` 的单元测试分工：
//! - 单元测试用显式路径注入 fake npm、纯函数判定，零进程全局副作用；
//! - 本文件走**完整 handler**：配方解析 → npm 在场探测（真读进程 PATH）→
//!   私有前缀安装（真读 `SEBAS_HOME`）→ 复核探测 → 建行判定（真读 agents 域
//!   快照）→ 目录 union（`GET /api/agents`），把 specs 的四条需求钉在真实
//!   HTTP 端点 + 真实状态库上：npm 缺失诚实报错 / 重复安装即升级 /
//!   config 种子受尊重 / 已有定义不被覆盖。
//!
//! **单独成文件**（同 `agent_store_probe_test.rs` 的理由）：要装进程级全局
//! 状态引擎 + 改进程 env（`SEBAS_HOME` / `PATH`），与同进程其它测试文件并行
//! 会互相污染；本文件内部再用一把静态锁把各用例串行化。
//!
//! Windows 不做硬验收（design D2/D7）——fake npm 是 unix sh 脚本，整文件
//! 仅 unix 编译。
#![cfg(unix)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use sebas_feishu::cards::CardConfig;
use sebas_webui::agent_kinds::{AgentKindInfo, AgentKindProvider};
use sebas_webui::models::RouterInfo;
use serde_json::Value;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use tower::ServiceExt;

/// 进程 env + 全局状态引擎的串行锁：每个用例独占，结束后还原 env。
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 一个用例的隔离环境：钉 `SEBAS_HOME`（安装前缀落点）+ 垫 fake npm 进
/// `PATH` + 装全新内存状态引擎。Drop 还原 env（字段按声明序 drop，env 还原
/// 在 `Drop::drop` 体内先发生）。
struct Fixture {
    _lock: MutexGuard<'static, ()>,
    _engine: sebas_dispatch::test_engine::EngineGuard,
    home: tempfile::TempDir,
    npm_dir: tempfile::TempDir,
    npm_log: PathBuf,
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl Fixture {
    /// `npm_present=false` → PATH 指向一个**没有 npm** 的目录（诚实报错场景）。
    fn new(npm_present: bool) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let engine = sebas_dispatch::test_engine::install_fresh();
        let home = tempfile::tempdir().unwrap();
        let npm_dir = tempfile::tempdir().unwrap();
        let npm_log = npm_dir.path().join("argv.log");
        let saved: Vec<(&'static str, Option<OsString>)> = ["SEBAS_HOME", "PATH"]
            .iter()
            .map(|v| (*v, std::env::var_os(v)))
            .collect();

        let path_value = if npm_present {
            // fake npm 目录前置 + 真实 PATH 兜底：脚本内的 mkdir/chmod 仍可解析。
            let real = std::env::var_os("PATH").unwrap_or_default();
            let mut p = npm_dir.path().as_os_str().to_os_string();
            p.push(":");
            p.push(&real);
            p
        } else {
            // 非空但无 npm 的目录。
            npm_dir.path().as_os_str().to_os_string()
        };
        unsafe {
            std::env::set_var("SEBAS_HOME", home.path());
            std::env::set_var("PATH", &path_value);
        }

        let fixture = Self {
            _lock: lock,
            _engine: engine,
            home,
            npm_dir,
            npm_log,
            saved,
        };
        if npm_present {
            fixture.write_fake_npm("ok", "9.9.9");
        }
        fixture
    }

    /// 写/覆写 fake npm 脚本（剧本与日志路径烘焙进脚本，不读进程 env）。
    fn write_fake_npm(&self, mode: &str, version: &str) {
        let script = self.npm_dir.path().join("npm");
        let body = format!(
            r#"#!/bin/sh
printf '%s\n' "$@" >> "{log}"
# npm 在场探测（handler 步骤 3 先跑 `npm --version`）：任何剧本都要诚实应答
# ——否则失败剧本会被误判成「npm 缺失」，走不到安装那一步。
if [ "$1" = "--version" ]; then
  echo "{version}"
  exit 0
fi
case "{mode}" in
  ok)
    # argv: install --global --prefix <prefix> <package>
    prefix="$4"
    pkg="$5"
    case "$pkg" in
      "@anthropic-ai/claude-code") name=claude ;;
      "opencode-ai") name=opencode ;;
      *) name=pkg ;;
    esac
    mkdir -p "$prefix/bin"
    printf '#!/bin/sh\necho {version}\n' > "$prefix/bin/$name"
    chmod +x "$prefix/bin/$name"
    exit 0
    ;;
  fail)
    echo "npm ERR! network unreachable" >&2
    echo "npm ERR! registry refused" >&2
    exit 7
    ;;
esac
exit 0
"#,
            log = self.npm_log.display(),
            mode = mode,
            version = version,
        );
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn argv_log(&self) -> &Path {
        &self.npm_log
    }

    fn argv(&self) -> Vec<String> {
        std::fs::read_to_string(&self.npm_log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for (var, prev) in &self.saved {
            match prev {
                Some(v) => unsafe { std::env::set_var(var, v) },
                None => unsafe { std::env::remove_var(var) },
            }
        }
    }
}

/// 空配置面 provider（config 种子为零）——`GET /api/agents` 的非 native 行
/// 只来自 store union。
fn empty_provider() -> Arc<dyn AgentKindProvider> {
    Arc::new(sebas_webui::agent_kinds::ConfigAgentKindProvider::new(
        Vec::new(),
    ))
}

/// 固定 id 的 config 种子 provider（不探测真实二进制，仅提供 id 集）。
struct SeedProvider {
    ids: Vec<String>,
}

#[async_trait::async_trait]
impl AgentKindProvider for SeedProvider {
    async fn agent_kinds(&self) -> Vec<AgentKindInfo> {
        self.ids
            .iter()
            .map(|id| AgentKindInfo {
                id: id.clone(),
                display: id.clone(),
                reachable: true,
                cause: None,
                version: None,
                display_raw: None,
            })
            .collect()
    }

    fn default_agent_kind(&self) -> String {
        "claude".to_string()
    }
}

/// 真实 handler + 真实状态库引擎（`InProcessBackend` 读全局 engine）装配。
/// 返回的 receiver 由调用方保活（防 `DispatchHandle::emit` 的 closed 断言）。
fn test_app(
    provider: Arc<dyn AgentKindProvider>,
) -> (
    axum::Router,
    tokio::sync::mpsc::Receiver<sebas_dispatch::engine::Out>,
) {
    let (router, rx) = DispatchHandle::new(SessionMap::new());
    let backend: Arc<dyn sebas_webui::SessionBackend> =
        Arc::new(sebas_webui::session_backend::InProcessBackend::new(router));
    let app = sebas_webui::build_router_with_agent_kind_provider(
        backend,
        RouterInfo::default(),
        CardConfig::default(),
        provider,
    );
    (app, rx)
}

async fn post_install(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents/install")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn agent_row<'a>(catalog: &'a Value, id: &str) -> &'a Value {
    catalog["agents"]
        .as_array()
        .unwrap_or_else(|| panic!("agents array missing: {catalog}"))
        .iter()
        .find(|a| a["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("agent '{id}' must be listed: {catalog}"))
}

async fn stored_ids() -> Vec<String> {
    let engine = sebas_dispatch::state_store::engine().expect("fixture engine");
    engine
        .load_agents()
        .await
        .expect("agents load")
        .into_iter()
        .map(|r| r.id)
        .collect()
}

// ── spec「一键安装到私有前缀」+「全新安装后目录行就绪」──────────────────────

#[tokio::test]
async fn install_opencode_seeds_a_reachable_catalog_row_under_the_private_prefix() {
    let f = Fixture::new(true);
    let (app, _rx) = test_app(empty_provider());

    let (status, v) = post_install(&app, serde_json::json!({"recipe": "opencode"})).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["installed"], true, "{v}");
    assert_eq!(
        v["agent_created"], true,
        "no prior definition → row created: {v}"
    );
    let expected_bin = f
        .home()
        .join("agent-tools")
        .join("opencode")
        .join("bin")
        .join("opencode");
    assert_eq!(
        v["path"].as_str(),
        Some(expected_bin.to_string_lossy().as_ref()),
        "response carries the private-prefix bin absolute path: {v}"
    );
    assert_eq!(
        v["version"], "9.9.9",
        "probed version rides the response: {v}"
    );
    assert!(
        expected_bin.starts_with(f.home()),
        "installed bin lives under the sandbox home: {expected_bin:?}"
    );

    // fake npm 收到的 argv（绝不真拨 registry 的证据面）。探测的
    // `--version` 也记进同一日志，故用「含」而非位置断言。
    let argv = f.argv();
    assert!(
        argv.iter().any(|a| a == "install"),
        "install subcommand rides argv: {argv:?}"
    );
    assert!(argv.iter().any(|a| a == "--global"), "{argv:?}");
    assert!(argv.iter().any(|a| a == "--prefix"), "{argv:?}");
    assert!(
        argv.iter().any(|a| a
            == &f
                .home()
                .join("agent-tools")
                .join("opencode")
                .to_string_lossy()
                .to_string()),
        "prefix must point inside the sandbox home: {argv:?}"
    );
    assert!(argv.iter().any(|a| a == "opencode-ai"), "{argv:?}");

    // 目录联动（spec「全新安装后目录行就绪」）：免重启即可见、可达。
    let (status, catalog) = get_json(&app, "/api/agents").await;
    assert_eq!(status, StatusCode::OK, "{catalog}");
    let row = agent_row(&catalog, "opencode");
    assert_eq!(row["reachable"], true, "installed row is reachable: {row}");
    assert_eq!(row["version"], "9.9.9", "{row}");
    assert_eq!(
        row["path_raw"].as_str(),
        Some(expected_bin.to_string_lossy().as_ref()),
        "catalog row points at the private bin: {row}"
    );
    assert_eq!(row["args"], serde_json::json!(["acp"]), "{row}");

    // 落库的行定义：acp 驱动、path 指私有 bin、sessions_dir/work_dir 留空。
    let engine = sebas_dispatch::state_store::engine().expect("engine");
    let rows = engine.load_agents().await.expect("agents load");
    let oc = rows
        .iter()
        .find(|r| r.id == "opencode")
        .expect("row landed");
    assert_eq!(oc.driver, "acp");
    assert_eq!(
        oc.args.as_deref(),
        Some("[\"acp\"]"),
        "args stored as a JSON list with the acp head: {oc:?}"
    );
    assert!(oc.sessions_dir.is_none(), "留空走默认: {oc:?}");
    assert!(oc.work_dir.is_none(), "留空走默认: {oc:?}");
}

// ── spec「npm 缺失诚实报错」────────────────────────────────────────────────

#[tokio::test]
async fn missing_npm_is_a_typed_400_with_node_guidance_and_no_store_change() {
    let f = Fixture::new(false);
    let (app, _rx) = test_app(empty_provider());

    let (status, v) = post_install(&app, serde_json::json!({"recipe": "claude"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let err = v["error"].as_str().unwrap_or_default();
    assert!(err.contains("npm"), "error names npm: {err}");
    assert!(
        err.contains("Node.js"),
        "error carries the minimal guidance: {err}"
    );

    // agents 目录零变化；安装前缀未创建。
    assert!(stored_ids().await.is_empty(), "no store row on npm-missing");
    assert!(
        !f.home().join("agent-tools").exists(),
        "no prefix dir is created when npm is missing"
    );
}

// ── spec「安装失败不落半成品」──────────────────────────────────────────────

#[tokio::test]
async fn npm_nonzero_exit_carries_stderr_tail_and_seeds_no_row() {
    let f = Fixture::new(true);
    f.write_fake_npm("fail", "9.9.9");
    let (app, _rx) = test_app(empty_provider());

    let (status, v) = post_install(&app, serde_json::json!({"recipe": "opencode"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let err = v["error"].as_str().unwrap_or_default();
    assert!(err.contains("npm 安装失败"), "{err}");
    assert!(
        err.contains("network unreachable") || err.contains("registry refused"),
        "stderr tail rides the failure: {err}"
    );
    assert!(
        stored_ids().await.is_empty(),
        "failure must not create a row"
    );
}

// ── spec「未知配方是 typed 拒绝」───────────────────────────────────────────

#[tokio::test]
async fn unknown_recipe_is_a_typed_400_and_spawns_no_subprocess() {
    let f = Fixture::new(true);
    let (app, _rx) = test_app(empty_provider());

    let (status, v) = post_install(&app, serde_json::json!({"recipe": "left-pad"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap_or_default()
            .contains("未知 recipe"),
        "{v}"
    );
    // 未 spawn：fake npm 的 argv 日志从未出现。
    assert!(!f.argv_log().exists(), "unknown recipe must not run npm");
    assert!(stored_ids().await.is_empty());
}

// ── spec「重复安装即升级」（design D6：无「已装拒绝」状态机）───────────────

#[tokio::test]
async fn reinstalling_the_same_recipe_upgrades_to_the_latest_version() {
    let f = Fixture::new(true);
    let (app, _rx) = test_app(empty_provider());

    // 首装：建行，版本 9.9.9。
    let (status, v) = post_install(&app, serde_json::json!({"recipe": "claude"})).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["agent_created"], true, "{v}");
    assert_eq!(v["version"], "9.9.9", "{v}");

    // 「npm」升级到新版本（重写假 bin 的版本），再次安装。
    f.write_fake_npm("ok", "10.0.0");
    let (status, v) = post_install(&app, serde_json::json!({"recipe": "claude"})).await;
    assert_eq!(status, StatusCode::OK, "reinstall is not a 409: {v}");
    assert_eq!(v["installed"], true, "{v}");
    assert_eq!(
        v["agent_created"], false,
        "the id already has a store row → not created again: {v}"
    );
    assert_eq!(v["version"], "10.0.0", "upgraded probe: {v}");

    // 目录刷新后版本为新探测值。
    let (_, catalog) = get_json(&app, "/api/agents").await;
    assert_eq!(
        agent_row(&catalog, "claude")["version"],
        "10.0.0",
        "{catalog}"
    );

    // 两次都真的执行了 npm install（幂等重装，无独立升级面）。
    let installs = f.argv().iter().filter(|a| *a == "install").count();
    assert_eq!(installs, 2, "both requests ran npm install: {:?}", f.argv());
}

// ── spec「已有定义不被覆盖」（store 行）───────────────────────────────────

#[tokio::test]
async fn existing_store_row_is_left_untouched() {
    let f = Fixture::new(true);
    // 操作员已在 store 手工维护 claude 行（自定义 path）。
    let engine = sebas_dispatch::state_store::engine().expect("engine");
    sebas_dispatch::state_store::agents_mutation(
        engine,
        &serde_json::json!({
            "op": "put",
            "id": "claude",
            "agent": {"driver": "claude", "path": "/bin/sh"}
        }),
    )
    .await
    .expect("seed manual claude row");

    let (app, _rx) = test_app(empty_provider());
    let (status, v) = post_install(&app, serde_json::json!({"recipe": "claude"})).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(
        v["installed"], true,
        "the install itself still succeeds: {v}"
    );
    assert_eq!(
        v["agent_created"], false,
        "an existing definition is never overwritten: {v}"
    );

    // 原行保持不动（自定义 path 未被私有前缀路径覆盖）。
    let (_, catalog) = get_json(&app, "/api/agents").await;
    assert_eq!(
        agent_row(&catalog, "claude")["path_raw"],
        "/bin/sh",
        "manual path survives the install: {catalog}"
    );
    let rows = engine.load_agents().await.expect("agents load");
    let claude = rows.iter().find(|r| r.id == "claude").expect("row");
    assert_eq!(claude.path.as_deref(), Some("/bin/sh"), "{claude:?}");
    let _ = f;
}

// ── spec「config 种子定义同受尊重」────────────────────────────────────────

#[tokio::test]
async fn config_seed_definition_is_respected_and_no_store_row_is_created() {
    let f = Fixture::new(true);
    let provider: Arc<dyn AgentKindProvider> = Arc::new(SeedProvider {
        ids: vec!["claude".to_string()],
    });
    let (app, _rx) = test_app(provider);

    let (status, v) = post_install(&app, serde_json::json!({"recipe": "claude"})).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["installed"], true, "{v}");
    assert_eq!(
        v["agent_created"], false,
        "a config-seeded id counts as an existing definition: {v}"
    );
    assert!(
        stored_ids().await.is_empty(),
        "no store row is created when the config seed already defines the id"
    );
    let _ = f;
}

// ── spec「并发同 recipe」端点级 409 ───────────────────────────────────────

#[tokio::test]
async fn concurrent_same_recipe_install_is_a_409() {
    let _f = Fixture::new(true);
    let (app, _rx) = test_app(empty_provider());
    let held = sebas_webui::agent_install::try_acquire("claude").expect("hold claude slot");
    let (status, v) = post_install(&app, serde_json::json!({"recipe": "claude"})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert!(
        v["error"].as_str().unwrap_or_default().contains("正在安装"),
        "{v}"
    );
    drop(held);
}

// ── 端点级守卫：外源 origin 403（与 agents CRUD / provider mutation 同款）───

#[tokio::test]
async fn non_loopback_origin_is_rejected_with_403() {
    let f = Fixture::new(true);
    let (app, _rx) = test_app(empty_provider());
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents/install")
                .header("content-type", "application/json")
                .header("origin", "http://evil.example")
                .body(Body::from(r#"{"recipe":"claude"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "cross-origin is refused"
    );
    // 守卫在任何落盘 / 子进程之前拒绝。
    assert!(!f.argv_log().exists(), "the guard runs before npm");
    assert!(stored_ids().await.is_empty());
}

// ── spec 场景「agent install endpoint runs the recipe installer」的角色半边：
//    SettingsManage 权限（root/admin 放行，member/viewer 403 且不 spawn）──────

/// 带鉴权 + 真实状态引擎的 app（登录走 cookie；install 端点在 RBAC 中央表
/// 挂 SettingsManage 档）。返回 (app, auth-db tempdir, auth handle)。
fn auth_app() -> (
    axum::Router,
    tempfile::TempDir,
    Arc<sebas_webui::auth::AuthHandle>,
) {
    let dir = tempfile::tempdir().unwrap();
    let auth = Arc::new(sebas_webui::auth::AuthHandle::open_with_iterations(
        dir.path().join("auth.db"),
        1000,
    ));
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let backend: Arc<dyn sebas_webui::SessionBackend> =
        Arc::new(sebas_webui::session_backend::InProcessBackend::new(router));
    let app = sebas_webui::build_router_with_auth(
        backend,
        RouterInfo::default(),
        CardConfig::default(),
        None,
        empty_provider(),
        30,
        auth.clone(),
    );
    (app, dir, auth)
}

fn test_addr() -> std::net::SocketAddr {
    std::net::SocketAddr::new(std::net::IpAddr::from([127, 0, 0, 1]), 12345)
}

async fn login_cookie(app: &axum::Router, user: &str, pass: &str) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("host", "127.0.0.1:12345")
                .header("content-type", "application/json")
                .extension(axum::extract::ConnectInfo(test_addr()))
                .body(Body::from(format!(
                    r#"{{"username":"{user}","password":"{pass}"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "login {user}");
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

#[tokio::test]
async fn install_requires_settings_manage_role() {
    let f = Fixture::new(true);
    let (app, _dir, auth) = auth_app();
    auth.setup_root("root", "password8").await.unwrap();
    let store = auth.user_store().unwrap();
    store
        .create("member", "password8", sebas_webui::rbac::Role::Member)
        .unwrap();
    store
        .create("viewer", "password8", sebas_webui::rbac::Role::Viewer)
        .unwrap();

    // member：认证通过但角色不足 → 403，且不触 npm、不落行。
    let member = login_cookie(&app, "member", "password8").await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents/install")
                .header("host", "127.0.0.1:12345")
                .header("content-type", "application/json")
                .header("cookie", &member)
                .extension(axum::extract::ConnectInfo(test_addr()))
                .body(Body::from(r#"{"recipe":"claude"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN, "member is role-gated");
    assert!(!f.argv_log().exists(), "role gate runs before npm");
    assert!(stored_ids().await.is_empty());

    // root：有 SettingsManage → 放行，安装成功。
    let root = login_cookie(&app, "root", "password8").await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents/install")
                .header("host", "127.0.0.1:12345")
                .header("content-type", "application/json")
                .header("cookie", &root)
                .extension(axum::extract::ConnectInfo(test_addr()))
                .body(Body::from(r#"{"recipe":"claude"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "root is allowed");
    assert!(
        stored_ids().await.contains(&"claude".to_string()),
        "root's install created the row"
    );
}
