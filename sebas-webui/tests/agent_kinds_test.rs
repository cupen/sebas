//! `GET /api/agent-kinds` — the create-session dropdown's reachable agent list.
//! Drives the endpoint in-process with a canned `AgentKindProvider` so the
//! shape is pinned without probing the host's real binaries.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use sebas_feishu::cards::CardConfig;
use sebas_webui::agent_kinds::{AgentKindInfo, AgentKindProvider};
use sebas_webui::build_router_with_agent_kind_provider;
use sebas_webui::models::RouterInfo;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

/// Canned provider: returns a fixed list, no subprocess probing.
struct CannedProvider {
    kinds: Vec<AgentKindInfo>,
    default_kind: String,
}

#[async_trait::async_trait]
impl AgentKindProvider for CannedProvider {
    async fn agent_kinds(&self) -> Vec<AgentKindInfo> {
        self.kinds.clone()
    }

    fn default_agent_kind(&self) -> String {
        self.default_kind.clone()
    }
}

fn info(id: &str, reachable: bool, cause: Option<&str>, version: Option<&str>) -> AgentKindInfo {
    AgentKindInfo {
        id: id.to_string(),
        display: id.to_string(),
        reachable,
        cause: cause.map(str::to_string),
        version: version.map(str::to_string),
        display_raw: None,
    }
}

async fn app_with(kinds: Vec<AgentKindInfo>) -> axum::Router {
    app_with_default(kinds, "claude".to_string()).await
}

async fn app_with_default(kinds: Vec<AgentKindInfo>, default_kind: String) -> axum::Router {
    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new(map);
    let backend: Arc<dyn sebas_webui::SessionBackend> =
        Arc::new(sebas_webui::session_backend::InProcessBackend::new(router));
    build_router_with_agent_kind_provider(
        backend,
        RouterInfo::default(),
        CardConfig::default(),
        Arc::new(CannedProvider {
            kinds,
            default_kind,
        }),
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
    let v: Value =
        serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("non-JSON body from {uri}: {e}"));
    (status, v)
}

#[tokio::test]
async fn agents_catalog_returns_canned_agents_with_optional_fields() {
    let app = app_with(vec![
        info("claude", true, None, Some("claude v2.1.0")),
        info("gemini", false, Some("command not found"), None),
    ])
    .await;

    let (status, v) = get_json(&app, "/api/agents").await;
    assert_eq!(status, StatusCode::OK);
    let agents = v["agents"].as_array().expect("agents array missing");
    // 2 个配置 agent + 1 行内置 native（catalog 是唯一真源，3.2）。
    assert_eq!(agents.len(), 3);
    let native = agents
        .iter()
        .find(|a| a["id"] == "native")
        .expect("native row");
    assert_eq!(native["display"], "Native Kernel");

    let claude = agents
        .iter()
        .find(|a| a["id"] == "claude")
        .expect("claude row");
    let gemini = agents
        .iter()
        .find(|a| a["id"] == "gemini")
        .expect("gemini row");
    let (claude, gemini) = (claude, gemini);

    assert_eq!(claude["id"], "claude");
    assert_eq!(claude["display"], "claude");
    assert_eq!(claude["reachable"], true);
    assert_eq!(claude["version"], "claude v2.1.0");
    assert!(
        claude.get("cause").is_none(),
        "reachable agent must omit cause"
    );
    // driver 是配置层概念，不上 wire（workbench-agent-wire-fix D3）。
    assert!(claude.get("driver").is_none(), "driver must not leak");
    assert!(claude.get("slug").is_none(), "slug is retired vocabulary");

    assert_eq!(gemini["id"], "gemini");
    assert_eq!(gemini["reachable"], false);
    assert_eq!(gemini["cause"], "command not found");
    assert!(
        gemini.get("version").is_none(),
        "unreachable agent must omit version"
    );
}

#[tokio::test]
async fn agents_catalog_still_lists_native_when_no_provider_entries() {
    let app = app_with(vec![]).await;
    let (status, v) = get_json(&app, "/api/agents").await;
    assert_eq!(status, StatusCode::OK);
    let agents = v["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 1, "native row is always present");
    assert_eq!(agents[0]["id"], "native");
}

/// preselect-last-used-model 3.2：`/api/about` 载荷携带装配点注入的
/// default agent kind 运行时真值（About INSTANCE 段只读行的数据源）。
#[tokio::test]
async fn about_payload_carries_injected_default_agent_kind() {
    let app = app_with_default(vec![], "codex".to_string()).await;
    let (status, v) = get_json(&app, "/api/about").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["default_agent_kind"], "codex", "about payload: {v}");
}

// ── fix-webui-qa-round10 1.1/1.2：store 行 agent 的可达性探测语义 ───────────
//
// QA round10 A-DEF-01 的验收合同（specs/agent-settings「Store-row
// reachability probe honors absolute paths」）：store 行（Settings 创建）与
// config 种子行对绝对路径同权——command[0] 是存在的绝对路径（正/反斜杠）
// 时必须 reachable，绝不对磁盘上存在的路径报 `command not found`。
//
// 证据核查结论（写入 tasks.md 备注）：QA 截图里表单填的是
// `C:/workbench/...`（反斜杠形态被自动化 fill 吞成 `C:workbench epos-...`），
// 而仓库与 fake-claude.exe 实际在 D: 盘——路径在磁盘上确实不存在，探测的
// 「command not found」是**诚实上报**。本模块把 spec 合同钉成回归测试：
// store 行 + 真实存在的绝对路径 → reachable（无需改组装代码）。
mod store_row_probe {
    use super::*;
    use sebas_webui::session_backend::FakeBackend;

    /// 平台无关的「确定存在且可执行」的绝对路径（探测目标不是本测试二进制
    /// 自身时不会触发 `--version` 子进程的奇怪语义——这里就用它自己）。
    fn existing_executable() -> std::path::PathBuf {
        if cfg!(windows) {
            std::env::current_exe().expect("test binary path")
        } else {
            std::path::PathBuf::from("/bin/sh")
        }
    }

    fn store_snapshot(id: &str, path: &str) -> serde_json::Value {
        // 形状 = AgentRow::snapshot_value（活跃行 + 墓碑 id 清单）。
        serde_json::json!({
            "agents": [{
                "id": id,
                "driver": "claude",
                "path": path,
                "args": ["--scenario", "error"],
                "source": "ui",
                "created_at": 0,
                "updated_at": 0,
            }],
            "deleted_ids": []
        })
    }

    async fn app_with_store_rows(snapshot: serde_json::Value) -> axum::Router {
        let map = SessionMap::new();
        let (router, _rx) = DispatchHandle::new(map);
        let backend: Arc<dyn sebas_webui::SessionBackend> =
            Arc::new(sebas_webui::session_backend::InProcessBackend::new(router));
        // config 种子面为空：catalog 里的非 native 行只来自 store union。
        let fake = FakeBackend::new();
        fake.set_state_domain("agents", Some(snapshot));
        build_router_with_agent_kind_provider(
            Arc::new(fake),
            RouterInfo::default(),
            CardConfig::default(),
            Arc::new(CannedProvider {
                kinds: vec![],
                default_kind: "claude".to_string(),
            }),
        )
    }

    /// spec 场景「GUI-created agent with an absolute Windows path is
    /// reachable」：store 行 + 存在的正斜杠绝对路径 → reachable，无 cause。
    #[tokio::test]
    async fn store_row_with_existing_forward_slash_path_is_reachable() {
        let exe = existing_executable();
        let forward = exe.to_string_lossy().replace('\\', "/");
        let app = app_with_store_rows(store_snapshot("claude-error", &forward)).await;
        let (status, v) = get_json(&app, "/api/agents").await;
        assert_eq!(status, StatusCode::OK);
        let row = v["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "claude-error")
            .expect("store row in catalog")
            .clone();
        assert_eq!(
            row["reachable"], true,
            "existing absolute path must probe reachable: {row}"
        );
        assert!(
            row.get("cause").is_none(),
            "reachable store row must omit cause: {row}"
        );
    }

    /// 反斜杠形态同权（Windows 专属——POSIX 里反斜杠是合法文件名字符，
    /// 「反斜杠路径」概念不成立）。
    #[cfg(windows)]
    #[tokio::test]
    async fn store_row_with_existing_backslash_path_is_reachable() {
        let exe = existing_executable();
        let app = app_with_store_rows(store_snapshot(
            "claude-error",
            &exe.to_string_lossy(),
        ))
        .await;
        let (_, v) = get_json(&app, "/api/agents").await;
        let row = v["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "claude-error")
            .expect("store row in catalog")
            .clone();
        assert_eq!(row["reachable"], true, "backslash form probes same: {row}");
    }

    /// 诚实性的另一半：store 行 + 磁盘上不存在的绝对路径 → 如实
    /// `command not found`（QA 观测到的正是这个正确行为）。
    #[tokio::test]
    async fn store_row_with_missing_absolute_path_reports_command_not_found() {
        let missing = if cfg!(windows) {
            r"Z:\definitely\not\here\fake-claude.exe".to_string()
        } else {
            "/definitely/not/here/fake-claude".to_string()
        };
        let app = app_with_store_rows(store_snapshot("claude-error", &missing)).await;
        let (_, v) = get_json(&app, "/api/agents").await;
        let row = v["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "claude-error")
            .expect("store row in catalog")
            .clone();
        assert_eq!(row["reachable"], false);
        assert_eq!(row["cause"], "command not found", "honest cause: {row}");
    }

    // 全链路（agents 域 mutation → 状态库 → catalog union → 探测）挪到
    // `agent_store_probe_test.rs`：它要装进程级全局状态引擎（install_fresh
    // 串行锁），与本文件里「无引擎」前提的 InProcessBackend 用例同跑会互相
    // 污染（全局引擎一装，catalog union 就读到别人的行）。
}
