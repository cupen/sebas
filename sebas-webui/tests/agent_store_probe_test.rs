//! fix-webui-qa-round10 1.1/1.2 — store 行 agent 的可达性探测全链路
//! （specs/agent-settings「Store-row reachability probe honors absolute
//! paths」的验收测试）。
//!
//! 单独成文件的缘故：链路要装进程级全局状态引擎（`install_fresh`，串行锁），
//! 与「无引擎」前提的其它测试文件同进程并行会互相污染——全局引擎一装，
//! `InProcessBackend::state_snapshot("agents")` 就读得到别人的行。
//!
//! 证据核查结论（QA round10 A-DEF-01）：QA 截图里表单填的是
//! `C:/workbench/...`（反斜杠形态被自动化 fill 吞成 `C:workbench epos-...`），
//! 而仓库与 fake-claude.exe 实际在 D: 盘——路径在磁盘上确实不存在，探测的
//! `command not found` 是诚实上报，不是缺陷。本文件把 spec 合同钉成回归
//! 测试：GUI 创建（store 行）+ 真实存在的绝对路径 → reachable，免重启。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_feishu::cards::CardConfig;
use sebas_webui::agent_kinds::{AgentKindInfo, AgentKindProvider};
use sebas_webui::build_router_with_agent_kind_provider;
use sebas_webui::models::RouterInfo;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

struct EmptyProvider;

#[async_trait::async_trait]
impl AgentKindProvider for EmptyProvider {
    async fn agent_kinds(&self) -> Vec<AgentKindInfo> {
        Vec::new()
    }

    fn default_agent_kind(&self) -> String {
        "claude".to_string()
    }
}

/// 平台无关的「确定存在且可执行」的绝对路径——用测试二进制自身（探测会对
/// 它跑 `--version`，libtest 对未知旗标非零退出也只是 version 缺席，
/// reachable 不受影响）。
fn existing_executable() -> std::path::PathBuf {
    if cfg!(windows) {
        std::env::current_exe().expect("test binary path")
    } else {
        std::path::PathBuf::from("/bin/sh")
    }
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

/// GUI 创建写路径 → 快照 → catalog union 的共享装配：真跑 `agents_mutation`
///（与 GUI 的 POST /api/agents 同一写通道），快照经 `AgentRow::snapshot_value`
///（InProcessBackend / core channel 服务端共用实现）注入 FakeBackend。
async fn app_after_gui_create(agent_payload: serde_json::Value) -> axum::Router {
    let _engine = sebas_dispatch::test_engine::install_fresh();
    let engine = sebas_dispatch::state_store::engine().expect("fixture engine");
    sebas_dispatch::state_store::agents_mutation(
        engine,
        &serde_json::json!({ "op": "put", "id": "claude-error", "agent": agent_payload }),
    )
    .await
    .expect("agents put");
    let rows = engine.load_agents().await.expect("agents load");
    let snapshot = sebas_models::agent::AgentRow::snapshot_value(&rows);
    let fake = sebas_webui::session_backend::FakeBackend::new();
    fake.set_state_domain("agents", Some(snapshot));
    build_router_with_agent_kind_provider(
        Arc::new(fake),
        RouterInfo::default(),
        CardConfig::default(),
        Arc::new(EmptyProvider),
    )
}

fn store_row<'a>(v: &'a Value) -> &'a Value {
    v["agents"]
        .as_array()
        .expect("agents array")
        .iter()
        .find(|a| a["id"] == "claude-error")
        .expect("store row survives mutation → snapshot → union")
}

/// spec 场景「GUI-created agent with an absolute Windows path is reachable」：
/// GUI 创建（store 行）+ 存在的正斜杠绝对路径 → reachable、无 cause，免重启。
#[tokio::test]
async fn gui_created_agent_with_existing_path_is_reachable_through_the_store() {
    let exe = existing_executable();
    let forward = exe.to_string_lossy().replace('\\', "/");
    let app = app_after_gui_create(serde_json::json!({
        "driver": "claude",
        "path": forward,
        "args": ["--scenario", "error"],
    }))
    .await;
    let (_, v) = get_json(&app, "/api/agents").await;
    let row = store_row(&v);
    assert_eq!(
        row["reachable"], true,
        "GUI-created agent with an existing path is reachable without restart: {row}"
    );
    assert!(row.get("cause").is_none(), "reachable row omits cause: {row}");
}

/// 反斜杠形态同权（Windows 专属——POSIX 里反斜杠是合法文件名字符，
/// 「反斜杠路径」概念不成立）。
#[cfg(windows)]
#[tokio::test]
async fn gui_created_agent_with_backslash_path_is_reachable() {
    let exe = existing_executable();
    let app = app_after_gui_create(serde_json::json!({
        "driver": "claude",
        "path": exe.to_string_lossy(),
    }))
    .await;
    let (_, v) = get_json(&app, "/api/agents").await;
    let row = store_row(&v);
    assert_eq!(row["reachable"], true, "backslash form probes same: {row}");
}

/// 诚实性的另一半：mutation 落了不存在的绝对路径（QA 观测的 C: 形态——
/// 仓库实际在 D:）→ catalog 如实 `command not found`，探测不虚报可达。
#[tokio::test]
async fn gui_created_agent_with_missing_path_stays_honestly_unreachable() {
    let missing = if cfg!(windows) {
        r"C:\workbench\repos-ai\sebas\target\debug\definitely-absent.exe".to_string()
    } else {
        "/definitely/not/here/fake-claude".to_string()
    };
    let app = app_after_gui_create(serde_json::json!({
        "driver": "claude",
        "path": missing,
    }))
    .await;
    let (_, v) = get_json(&app, "/api/agents").await;
    let row = store_row(&v);
    assert_eq!(row["reachable"], false, "honest probe: {row}");
    assert_eq!(row["cause"], "command not found", "honest cause: {row}");
}
