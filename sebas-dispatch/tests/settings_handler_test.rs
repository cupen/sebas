//! `/settings` 处理器：列表 / 修改 / 非法值拒绝。
//!
//! retire-legacy-state-json 4.3：card 设置的持久化**只走状态库**——
//! `settings.json` 已退休，本文件不再写任何文件，改用进程内内存引擎
//! （`sebas_dispatch::test_engine`）验证「落到库」与「库不可用时拒绝写」。

use sebas_channels::ChannelKey;
use sebas_dispatch::cards::{CardConfig, ThinkingDisplay};
use sebas_dispatch::engine::{DispatchHandle, Out};
use sebas_dispatch::state::SessionMap;

fn key() -> ChannelKey {
    ChannelKey::feishu("oc_test", None)
}

async fn next_out(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Out {
    rx.recv().await.expect("expected Out")
}

#[tokio::test]
async fn settings_list_emits_all_keys() {
    let _engine = sebas_dispatch::test_engine::install_fresh();
    let (router, mut rx) = DispatchHandle::new(SessionMap::new());

    router.handle_settings(key(), None, None).await;
    let out = next_out(&mut rx).await;
    let Out::PlainText { key: _k, content } = out else {
        panic!("expected PlainText, got {out:?}");
    };
    assert!(
        content.contains("thinking"),
        "missing thinking in list: {content}"
    );
    assert!(
        content.contains("show"),
        "default thinking not shown: {content}"
    );
    // 来源不再是一个文件路径，而是状态库的 card_config 键。
    assert!(
        content.contains("状态库"),
        "list must name the state store as the source: {content}"
    );
}

/// 修改后：回复确认、内存快照更新、**值落到状态库**（settings 表
/// `card_config` 键，经 `StateStoreEngine::save_settings`）。
#[tokio::test]
async fn settings_set_persists_to_the_state_store() {
    let _engine = sebas_dispatch::test_engine::install_fresh();
    let (router, mut rx) = DispatchHandle::new(SessionMap::new());

    router
        .handle_settings(key(), Some("thinking".into()), Some("hide".into()))
        .await;
    let out = next_out(&mut rx).await;
    let Out::PlainText { content, .. } = out else {
        panic!("expected PlainText, got {out:?}");
    };
    assert!(
        content.contains("hide"),
        "expected hide in reply: {content}"
    );

    // 运行期内存快照已更新。
    let cfg = router.card_config().await;
    assert_eq!(cfg.thinking, ThinkingDisplay::Hide);

    // 库里落的是全量快照（与内存快照同值），且没有任何文件参与。
    let engine = sebas_dispatch::state_store::engine().expect("engine installed");
    let stored = engine
        .load_settings()
        .await
        .expect("load settings")
        .expect("settings row written");
    let stored: CardConfig = serde_json::from_value(stored).expect("stored value is a CardConfig");
    assert_eq!(stored.thinking, ThinkingDisplay::Hide);
}

/// 状态库不可用（无引擎）时：**拒绝写入并如实回报**，既不留文件也不假装
/// 已保存（spec「reading clients no longer fall back to a file」）。
#[tokio::test]
async fn settings_set_reports_unavailable_when_store_is_missing() {
    // 不装引擎：显式清空，回到「状态库不可用」姿态（与 install_fresh 共用
    // 同一把全局锁，因此不会与其它用例抢引擎槽）。
    let _engine = sebas_dispatch::test_engine::install_none();
    let (router, mut rx) = DispatchHandle::new(SessionMap::new());

    router
        .handle_settings(key(), Some("thinking".into()), Some("hide".into()))
        .await;
    let out = next_out(&mut rx).await;
    let Out::PlainText { content, .. } = out else {
        panic!("expected PlainText, got {out:?}");
    };
    assert!(
        content.contains("保存失败") && content.contains("不可用"),
        "库不可用必须如实拒绝: {content}"
    );
}

#[tokio::test]
async fn settings_rejects_invalid_value() {
    let _engine = sebas_dispatch::test_engine::install_fresh();
    let (router, mut rx) = DispatchHandle::new(SessionMap::new());

    router
        .handle_settings(key(), Some("thinking".into()), Some("disable".into()))
        .await;
    let out = next_out(&mut rx).await;
    let Out::PlainText { content, .. } = out else {
        panic!("expected PlainText");
    };
    assert!(
        content.contains("可选值") || content.contains("show"),
        "expected validation error, got {content}"
    );
}