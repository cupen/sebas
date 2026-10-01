//! fix-webui-qa-round7 2.2（acp-model-selection D2）的引擎级单测。
//!
//! 覆盖（纯引擎路径，不拉子进程；pump 归类半边在根 crate `tests/pump_unit_test.rs`）：
//! - 模型切换拒绝（非终态 Error + `MODEL_UNCHANGED_MARKER`）走
//!   `apply_event_to_out` 的终态边：SEED/WORKING 锚定收尾 DONE，拒绝条目
//!   可见，占位（空 prompt 的 SEED）绝不被推成 WORKING 滞留（QA DEF-02）；
//! - `cancelled_turns` 按回合身份消费：被打标回合若经拒绝收尾（无
//!   Finished），陈旧标记不得注入后续无关回合的呈现（虚假「操作者中断」）；
//! - 无回合可标（从未开轮）的 `/cancel` 不打标。

use sebas_acp::claude::session::AcpEvent;
use sebas_acp::MODEL_UNCHANGED_MARKER;
use sebas_channels::ChannelKey;
use sebas_dispatch::card_state::phase;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::{Mapping, SessionMap};
use sebas_dispatch::TurnEntry;

fn web_key(tag: &str) -> ChannelKey {
    ChannelKey::new("web", format!("round7-{tag}"))
}

/// 相位快照直读（同 approval_restore_identity_test 的公开口径）。
async fn card_phase(router: &DispatchHandle, session_id: &str) -> Option<String> {
    router
        .card_state_snapshot()
        .await
        .remove(session_id)
        .map(|st| st.status_emoji)
}

fn rejection_message(model: &str) -> String {
    format!("set model {model:?} 被拒绝（Invalid params: \"invalid model id\"），{MODEL_UNCHANGED_MARKER}")
}

/// 主契约：接收回执相位（SEED + prompt）收到模型拒绝 → 回合立即以终态收尾
/// （DONE），拒绝条目可见，不滞留 WORKING。
#[tokio::test]
async fn model_rejection_settles_the_engaged_turn_immediately() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("settle");
    router
        .map
        .insert(key.clone(), Mapping::active("s-settle"))
        .await
        .unwrap();
    router.seed_card("s-settle".to_string(), "run".into()).await;
    assert_eq!(
        card_phase(&router, "s-settle").await.as_deref(),
        Some(phase::SEED),
        "前置：回合处于接收回执相位"
    );

    router
        .apply_event_to_out(
            "s-settle".into(),
            &AcpEvent::Error {
                session_id: "s-settle".into(),
                message: rejection_message("bad-model"),
                terminal: false,
            },
        )
        .await;

    assert_eq!(
        card_phase(&router, "s-settle").await.as_deref(),
        Some(phase::DONE),
        "拒绝即终态：SEED 锚定收尾 DONE，绝不滞留 WORKING"
    );
    let turns = router.session_turns(&key, 0).await.unwrap();
    assert!(
        turns
            .iter()
            .any(|e| e.element_type == "error".into() && e.content.contains("bad-model")),
        "拒绝回执如实上屏（错误条目）"
    );
    assert!(
        !turns.iter().any(|e| e.content.contains("回合被停止")),
        "拒绝收尾不是操作者中断，无停止条目"
    );
}

/// 占位幽灵回合（空 prompt 的 SEED，从未开轮）收到模型拒绝：同样收尾 DONE，
/// 绝不推进 WORKING（design D2「无真实回合在跑时引擎不得推进相位」）。
#[tokio::test]
async fn model_rejection_never_pushes_a_placeholder_into_working() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("ghost");
    router
        .map
        .insert(key.clone(), Mapping::active("s-ghost"))
        .await
        .unwrap();
    // 激活语义的空 prompt 种子（聚焦即拉起）——没有回合在跑。
    router.seed_card("s-ghost".to_string(), String::new()).await;
    assert_eq!(card_phase(&router, "s-ghost").await.as_deref(), Some(phase::SEED));

    router
        .apply_event_to_out(
            "s-ghost".into(),
            &AcpEvent::Error {
                session_id: "s-ghost".into(),
                message: rejection_message("bad-model"),
                terminal: false,
            },
        )
        .await;

    let settled = card_phase(&router, "s-ghost").await;
    assert_eq!(
        settled.as_deref(),
        Some(phase::DONE),
        "占位回合被拒绝事件直接终态化，不经 WORKING"
    );
}

/// 虚假中断主契约：回合 1 被操作者取消打标，但经**模型拒绝**收尾（无
/// Finished）——陈旧标记必须原样过期，回合 2 正常完成时绝不出现
/// 「操作者中断」条目（QA DEF-02 的第二症状）。
#[tokio::test]
async fn stale_cancel_marker_never_annotates_a_later_turn() {
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("stale");
    router
        .map
        .insert(key.clone(), Mapping::active("s-stale"))
        .await
        .unwrap();

    // 回合 1：开轮 → 流式中 → 操作者点停止（打标，锚 = prompt p1）。
    router.seed_card("s-stale".to_string(), "first".into()).await;
    router
        .dispatch_acp_event(AcpEvent::TextDelta {
            session_id: "s-stale".into(),
            delta: "streaming".into(),
        })
        .await;
    assert!(matches!(
        router.web_cancel_session(&key).await,
        sebas_dispatch::engine::CancelOutcome::Dispatched
    ));
    while let Ok(Some(_)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), out_rx.recv()).await
    {
        // 排空 cancel 派发产生的 Out，避免后续断言被无关指令干扰。
        if out_rx.is_empty() {
            break;
        }
    }

    // 回合 1 未经 Finished 收尾：模型拒绝错误直接终态化（本轮修复的路径）。
    router
        .apply_event_to_out(
            "s-stale".into(),
            &AcpEvent::Error {
                session_id: "s-stale".into(),
                message: rejection_message("bad-model"),
                terminal: false,
            },
        )
        .await;
    assert_eq!(card_phase(&router, "s-stale").await.as_deref(), Some(phase::DONE));

    // 回合 2：正常新回合。
    router
        .web_send_message(key.clone(), "second".into())
        .await
        .expect("accepted");
    router
        .dispatch_acp_event(AcpEvent::TextDelta {
            session_id: "s-stale".into(),
            delta: "turn two output".into(),
        })
        .await;
    router
        .apply_event(
            "s-stale",
            &AcpEvent::Finished {
                session_id: "s-stale".into(),
            },
        )
        .await;

    let turns = router.session_turns(&key, 0).await.unwrap();
    assert!(
        !turns.iter().any(|e| e.content.contains("回合被停止")),
        "陈旧取消标记不得注入后续回合：无「操作者中断」条目"
    );
    assert!(
        turns.iter().any(|e| e.content.contains("turn two output")),
        "回合 2 的真实内容完整保留"
    );
}

/// 无回合可标的取消（从未开轮的会话发 `/cancel`）不打标：后续首回合正常
/// 完成不含停止条目。
#[tokio::test]
async fn cancel_without_a_started_turn_leaves_no_marker() {
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("idle-cancel");
    router
        .map
        .insert(key.clone(), Mapping::active("s-idle"))
        .await
        .unwrap();

    // 空闲会话的 /cancel 命令路径：无 prompt → 无锚 → 不打标。
    router
        .web_send_message(key.clone(), "/cancel".into())
        .await
        .expect("accepted");
    while let Ok(Some(_)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), out_rx.recv()).await
    {
        if out_rx.is_empty() {
            break;
        }
    }

    // 之后的首个真实回合正常完成。
    router.seed_card("s-idle".to_string(), "hello".into()).await;
    router
        .dispatch_acp_event(AcpEvent::TextDelta {
            session_id: "s-idle".into(),
            delta: "answer".into(),
        })
        .await;
    router
        .apply_event(
            "s-idle",
            &AcpEvent::Finished {
                session_id: "s-idle".into(),
            },
        )
        .await;

    let turns = router.session_turns(&key, 0).await.unwrap();
    assert!(
        !turns.iter().any(|e| e.content.contains("回合被停止")),
        "无事可停的取消不得伪造停止条目"
    );
}

/// 对照组（同回合锚定仍生效）：被打标回合**自身**的 Finished 照旧注入停止
/// 条目——真实中断路径（审批等待中点停止同锚）不得被身份收紧误伤。
#[tokio::test]
async fn a_marked_turn_still_annotates_its_own_finish() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("real-interrupt");
    router
        .map
        .insert(key.clone(), Mapping::active("s-real"))
        .await
        .unwrap();
    router.seed_card("s-real".to_string(), "run".into()).await;
    router
        .dispatch_acp_event(AcpEvent::TextDelta {
            session_id: "s-real".into(),
            delta: "streaming".into(),
        })
        .await;
    assert!(matches!(
        router.web_cancel_session(&key).await,
        sebas_dispatch::engine::CancelOutcome::Dispatched
    ));
    router
        .apply_event(
            "s-real",
            &AcpEvent::Finished {
                session_id: "s-real".into(),
            },
        )
        .await;

    let turns = router.session_turns(&key, 0).await.unwrap();
    let stops: Vec<&TurnEntry> = turns
        .iter()
        .filter(|e| e.element_type == "error".into() && e.content.contains("回合被停止"))
        .collect();
    assert_eq!(stops.len(), 1, "真实中断仍如实标注一条停止条目");
}
