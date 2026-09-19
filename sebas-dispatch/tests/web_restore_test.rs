//! 归档恢复重建会话的引擎级单测（fix-webui-qa-defects 2.1，design D1）。
//!
//! 契约：`web_restore_session` 以原 key 重建 Dormant 映射并把转写回放进
//! turn 存储——恢复后 `session_info_snapshot` 含该会话、`session_turns`
//! 返回全部 N 条条目；key 已有映射时拒绝（绝不覆盖活状态）；resume 回退
//! 新路由 id 时旧转写随激活迁移。

use sebas_channels::ChannelKey;
use sebas_dispatch::engine::failure_class;
use sebas_dispatch::state::{Mapping, SessionMap};
use sebas_dispatch::{DispatchHandle, SessionIdentity, TurnEntry};

fn web_key(id: &str) -> ChannelKey {
    ChannelKey::new("web", format!("web-{id}"))
}

fn sample_transcript() -> Vec<TurnEntry> {
    vec![
        TurnEntry::prompt(0, "hello"),
        TurnEntry::markdown(1, "world"),
        TurnEntry::error(2, "**spawn failed**: boom").with_failure_class(failure_class::SPAWN),
    ]
}

/// 2.1 主契约：恢复后快照含该会话（dormant）、detail 返回全部 N 条条目，
/// 且首条消息走 resume 语义（Dormant 被认领、交还原 session_id）。
#[tokio::test]
async fn restore_rebuilds_dormant_mapping_with_full_transcript() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("r1");
    let transcript = sample_transcript();

    router
        .web_restore_session(
            key.clone(),
            Some("old-sid".to_string()),
            Some("/proj".to_string()),
            transcript,
            SessionIdentity::default(),
        )
        .await
        .expect("restore must succeed");

    // 快照可见，状态 dormant，project_dir 沿用。
    let snap = router.session_info_snapshot().await;
    assert_eq!(snap.len(), 1, "restored session must appear in the snapshot");
    assert_eq!(snap[0].status, "dormant");
    assert_eq!(snap[0].project_dir.as_deref(), Some("/proj"));
    assert_eq!(snap[0].session_id.as_deref(), Some("old-sid"));

    // detail（turns）完整：全部 N 条条目按序可见。
    let turns = router.session_turns(&key, 0).await.expect("mapping exists");
    assert_eq!(turns.len(), 3, "all archived entries must be visible");
    assert_eq!(turns[0].kind, "prompt");
    assert_eq!(turns[0].content, "hello");
    assert_eq!(turns[1].content, "world");
    // 错误条目的失败分类随回放保留。
    assert_eq!(turns[2].failure_class.as_deref(), Some(failure_class::SPAWN));

    // 首条消息 = resume：Dormant 被认领，交还原 session_id（agent load 失败
    // 时由既有回退语义兜底）。
    let route = router
        .map
        .route_text(key.clone(), "continue".into())
        .await
        .unwrap();
    match route {
        sebas_dispatch::state::TextRoute::Resume(old) => {
            assert_eq!(old, "old-sid", "resume must target the original session id");
        }
        other => panic!("first message must route as Resume, got {other:?}"),
    }
}

/// 归档条目无原 session_id（旧 archive.json）：以 key reference 合成
/// 确定性 id，transcript 寻址不受影响。
#[tokio::test]
async fn restore_without_session_id_synthesizes_one_from_the_key() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("r2");
    router
        .web_restore_session(key.clone(), None, None, sample_transcript(), SessionIdentity::default())
        .await
        .expect("restore must succeed");
    let turns = router.session_turns(&key, 0).await.expect("mapping exists");
    assert_eq!(turns.len(), 3, "transcript addressing must not need the old id");
    let snap = router.session_info_snapshot().await;
    assert_eq!(snap[0].session_id.as_deref(), Some(key.reference.as_str()));
}

/// 已有映射的 key 拒绝重建（活会话/占位与归档同 key 是状态矛盾）——
/// 绝不覆盖在用状态。
#[tokio::test]
async fn restore_rejects_a_key_that_already_has_a_mapping() {
    let map = SessionMap::new();
    let key = web_key("r3");
    map.insert(key.clone(), Mapping::active("live-sid"))
        .await
        .unwrap();
    let (router, _rx) = DispatchHandle::new(map);

    let err = router
        .web_restore_session(key.clone(), None, None, sample_transcript(), SessionIdentity::default())
        .await;
    assert!(err.is_err(), "an existing mapping must block the restore");
    // 活映射未被触碰。
    let m = router.map.get(&key).await.unwrap();
    assert_eq!(m.session_id(), Some("live-sid"));
    assert!(
        router.session_turns(&key, 0).await.unwrap().is_empty(),
        "no transcript may be replayed over a live session"
    );
}

/// 空转写的归档条目（旧格式 / 零对话）也能恢复：映射在、turns 为空。
#[tokio::test]
async fn restore_with_empty_transcript_still_rebuilds_the_mapping() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("r4");
    router
        .web_restore_session(key.clone(), Some("sid-4".into()), None, Vec::new(), SessionIdentity::default())
        .await
        .expect("restore must succeed");
    let info = router.session_info_for(&key).await.expect("mapping exists");
    assert_eq!(info.status, "dormant");
    assert!(router.session_turns(&key, 0).await.unwrap().is_empty());
}

/// resume 回退新路由 id 时，旧 id 名下的转写随激活迁移（对话历史不因
/// 路由 id 更换而「消失」）。
#[tokio::test]
async fn activation_migrates_transcript_to_the_new_routing_id() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("r5");
    router
        .web_restore_session(key.clone(), Some("old-5".into()), None, sample_transcript(), SessionIdentity::default())
        .await
        .expect("restore must succeed");
    // dispatcher 的 resume 完成：activate 以新路由 id 翻正映射。
    router.activate(&key, "new-5".into(), None, None).await;
    let turns = router
        .session_turns(&key, 0)
        .await
        .expect("mapping exists after activation");
    assert_eq!(turns.len(), 3, "history must follow the new routing id");
    assert_eq!(turns[0].content, "hello", "order preserved after migration");
}
