//! 路由 id ↔ 真实 ACP session id 映射的持久化语义
//! (openspec/changes/add-acp-session-id-mapping，spec 场景 1–3)：
//!
//! - 建 ACP 会话 → 映射写入 state（`dump_json` 携带 `acp_session_id`）；
//! - 旧 state 无 `acp_session_id` → 读为 `None` 不报错；
//! - 无映射/resume load 被拒 → 诚实回退 fresh（`resumed=false`）；
//! - load 失败 → 原映射保留在存储（D4），新会话以新 routing id 落地。
//!
//! 全过程走 `sebas::run::acp_spawn_and_activate` / `acp_resume_and_activate`
//! （生产调用面） + 真实 `fake-acp-agent` mock（`session/new` 回
//! `acp-new-<nanoid>`，`load-ok` 接受任意 load id，`load-fails` 拒绝）。

use sebas_acp::claude::manager::SessionManager;
use sebas_feishu::events::{FeishuIn, SessionKey};
use sebas_router::router::{Out, RouterHandle};
use sebas_router::state::{MappingState, SessionMap};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn fake_acp() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/debug")
        .join(format!("fake-acp-agent{}", std::env::consts::EXE_SUFFIX))
}

fn key() -> SessionKey {
    SessionKey {
        chat_id: "oc_acp_map".into(),
        thread_id: None,
    }
}

fn acp_manager() -> SessionManager {
    let mut agents = HashMap::new();
    agents.insert(
        "acp".to_string(),
        sebas_acp::claude::manager::AgentEntry {
            driver: Arc::new(sebas_acp::AcpDriver),
            startup_timeout: Duration::from_secs(10),
        },
    );
    SessionManager::new("acp".to_string(), agents)
}

/// 生产调用面：以定位 `fake-acp-agent` 的 command 建一个 fresh ACP 会话，
/// 返回 routing id（会话仍活，调用方负责 kill）。
async fn spawn_fresh(
    mgr: &Arc<SessionManager>,
    router: &RouterHandle,
    map: &SessionMap,
    scenario: &str,
) -> String {
    let (sid, _pending, _rx, _model_info) = sebas::run::acp_spawn_and_activate(
        mgr,
        router,
        &key(),
        "hi".into(),
        "acp",
        vec![fake_acp().to_string_lossy().into_owned(), scenario.into()],
        None,
        None,
        None)
    .await
    .expect("fresh acp spawn must succeed");
    assert_eq!(
        map.get(&key()).await.unwrap().session_id(),
        Some(sid.as_str()),
        "mapping active under the routing id"
    );
    sid
}

/// 生产调用面：resume 一个 persisted 会话（`load-fails` 场景保证回退）。
#[allow(clippy::type_complexity)]
async fn resume(
    mgr: &Arc<SessionManager>,
    router: &RouterHandle,
    old_sid: &str,
    scenario: &str,
) -> (String, bool) {
    let (sid, _pending, _rx, resumed) = sebas::run::acp_resume_and_activate(
        mgr,
        router,
        &key(),
        old_sid,
        "hi".into(),
        "acp",
        vec![fake_acp().to_string_lossy().into_owned(), scenario.into()],
        None,
        None,
        None)
    .await
    .expect("resume/fallback must not error");
    (sid, resumed)
}

#[tokio::test]
async fn acp_spawn_persists_real_session_id_mapping() {
    let map = SessionMap::new();
    let (router, _out_rx) = RouterHandle::new(map.clone());
    let mgr = Arc::new(acp_manager());

    let sid = spawn_fresh(&mgr, &router, &map, "load-ok").await;
    let real = mgr.get_acp_session_id(&sid).await.expect("driver reports id");
    assert!(
        real.starts_with("acp-new-"),
        "fresh spawn reports the real session/new id, got {real:?}"
    );
    // 映射已随 activate 写入内存。
    assert_eq!(
        map.get(&key()).await.unwrap().acp_session_id.as_deref(),
        Some(real.as_str()),
        "routing id ↔ real ACP session id persisted on the mapping"
    );
    // dump_json 落盘携带 acp_session_id（重启后 resume 可读）。
    let json = map.dump_json().await.unwrap();
    assert!(
        json.contains(&format!("\"acp_session_id\":\"{real}\"")),
        "state dump carries acp_session_id, got: {json}"
    );

    mgr.kill(&sid).await;
}

#[tokio::test]
async fn resume_without_mapped_session_id_falls_back_to_fresh() {
    // 旧记录：有 routing id、无 acp_session_id（上库版本/无独立 id 的 agent）。
    // resume 时驱动用 routing id 尝试 load；agent 拒绝（load-fails）→
    // 诚实回退 fresh（resumed=false、新 routing id + 新真实 id）。
    let json = r#"{"oc_acp_map":{"session_id":"s-gone","last_active_unix":1}}"#;
    let map = SessionMap::restore_json(json).unwrap();
    let (router, _out_rx) = RouterHandle::new(map.clone());
    let mgr = Arc::new(acp_manager());

    let (sid, resumed) = resume(&mgr, &router, "s-gone", "load-fails").await;

    assert!(!resumed, "no mapping + rejected load → resumed=false");
    assert_ne!(sid, "s-gone", "fallback mints a fresh routing id");
    let fresh_real = mgr.get_acp_session_id(&sid).await.expect("driver reports id");
    assert!(
        fresh_real.starts_with("acp-new-"),
        "fallback-fresh records a new session/new id, got {fresh_real:?}"
    );
    // 映射激活在新的 routing id 上（fallback 不是 fail_spawn）。
    assert_eq!(
        map.get(&key()).await.unwrap().session_id(),
        Some(sid.as_str())
    );

    mgr.kill(&sid).await;
}

#[tokio::test]
async fn acp_resume_uses_mapped_real_id_after_restart() {
    // 建会话 → dump → restore（模拟 daemon 重启）→ resume：
    // 映射的 acp_session_id 让 resume 用真实 id 发 session/load（非 routing id）。
    let map = SessionMap::new();
    let (router, _out_rx) = RouterHandle::new(map.clone());
    let mgr = Arc::new(acp_manager());

    let sid = spawn_fresh(&mgr, &router, &map, "load-ok").await;
    let real = mgr.get_acp_session_id(&sid).await.unwrap();
    mgr.kill(&sid).await; // 会话结束；映射仍是 Active，dump 可持久化。
    let json = map.dump_json().await.unwrap();
    assert!(json.contains(&format!("\"acp_session_id\":\"{real}\"")));

    // 重启：restore 后映射成 Dormant，acp_session_id 保留。
    let map2 = SessionMap::restore_json(&json).unwrap();
    let m = map2.get(&key()).await.unwrap();
    assert!(matches!(m.state, MappingState::Dormant { .. }));
    assert_eq!(m.acp_session_id.as_deref(), Some(real.as_str()));

    // 第一次入站文本 → SpawnResume（Dormant 被 claim）。
    let mgr2 = Arc::new(acp_manager());
    let (router2, mut out_rx) = RouterHandle::new(map2.clone());
    router2
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "继续".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;
    let out = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
        .await
        .expect("out within 500ms")
        .expect("channel open");
    let Out::SpawnResume {
        key: k,
        session_id: old,
        prompt,
        ..
    } = out
    else {
        panic!("expected SpawnResume")
    };
    assert_eq!(old, sid);
    assert_eq!(prompt, "继续");

    // load-ok mock + 提供真实 id → resumed=true、routing id 保留。
    let (sid2, pending, rx, resumed) = sebas::run::acp_resume_and_activate(
        &mgr2,
        &router2,
        &k,
        &old,
        &prompt,
        "acp",
        vec![fake_acp().to_string_lossy().into_owned(), "load-ok".into()],
        None,
        None,
        None)
    .await
    .expect("resume ok");
    assert!(resumed, "resume with mapped real id succeeds");
    assert_eq!(sid2, sid, "routing id preserved on resumed load");
    assert_eq!(
        mgr2.get_acp_session_id(&sid2).await.as_deref(),
        Some(real.as_str()),
        "manager re-records the loaded ACP session id"
    );
    drop((pending, rx));

    mgr2.kill(&sid2).await;
}

#[tokio::test]
async fn load_failure_keeps_original_mapping_in_state() {
    // 建会话（映射含真实 id）→ 重启 → resume 时 agent 拒绝 load：
    // 回退 fresh 成功，同时原映射（旧 routing id ↔ 旧 acp_session_id）
    // 以 dormant 归档记录保留在存储（D4：旧会话仍可被未来 load 寻址，
    // 不因一次失败而抹除）。
    let map = SessionMap::new();
    let (router, _out_rx) = RouterHandle::new(map.clone());
    let mgr = Arc::new(acp_manager());

    let sid = spawn_fresh(&mgr, &router, &map, "load-ok").await;
    let real = mgr.get_acp_session_id(&sid).await.unwrap();
    mgr.kill(&sid).await;
    let json = map.dump_json().await.unwrap();
    let map2 = SessionMap::restore_json(&json).unwrap();

    let mgr2 = Arc::new(acp_manager());
    let (router2, _out_rx2) = RouterHandle::new(map2.clone());
    let (sid2, resumed) = resume(&mgr2, &router2, &sid, "load-fails").await;

    assert!(!resumed, "rejected load → resumed=false");
    assert_ne!(sid2, sid, "fallback mints a fresh routing id");

    // 新映射落地（新 routing id → 新真实 id）。
    let m_new = map2.get(&key()).await.unwrap();
    assert_eq!(m_new.session_id(), Some(sid2.as_str()));
    let new_real = mgr2.get_acp_session_id(&sid2).await.unwrap();
    assert!(
        new_real.starts_with("acp-new-") && new_real != real,
        "rejected real id ({real:?}) must NOT be re-recorded; new session has a fresh one: {new_real:?}"
    );
    assert_eq!(m_new.acp_session_id.as_deref(), Some(new_real.as_str()));

    // D4：原映射作为 dormant 归档记录保留在存储（closed-* 键），dump 里
    // 同时存在新会话记录与旧 routing id 的归档记录。
    let json_after = map2.dump_json().await.unwrap();
    assert!(
        json_after.contains(&format!("\"acp_session_id\":\"{real}\"")),
        "old real id preserved in storage, got: {json_after}"
    );
    assert!(
        json_after.contains("\"closed-"),
        "old routing id archived under a closed-* dormant key, got: {json_after}"
    );
    assert!(
        json_after.contains(&format!("\"session_id\":\"{sid}\"")),
        "old routing id still addressable in storage, got: {json_after}"
    );

    mgr2.kill(&sid2).await;
}

#[tokio::test]
async fn legacy_state_without_acp_session_id_restores_as_none() {
    // 旧 state.json（无 acp_session_id 字段）读为 None、不报错
    // （tasks 2.1 验证项；serde default）。
    let json = r#"{"oc_legacy":{"session_id":"s-legacy","last_active_unix":7}}"#;
    let map = SessionMap::restore_json(json).unwrap();
    let m = map
        .get(&SessionKey {
            chat_id: "oc_legacy".into(),
            thread_id: None,
        })
        .await
        .unwrap();
    assert_eq!(m.acp_session_id, None);
    // round-trip：dump 后再 restore 依旧合法。
    let json2 = map.dump_json().await.unwrap();
    let _ = SessionMap::restore_json(&json2).unwrap();
}