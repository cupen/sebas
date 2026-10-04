//! 会话级用量投影的引擎级单测（add-webui-round7-gaps 3c 修订）。
//!
//! 合同（usage-statistics delta「会话级 token 用量可见」）：
//! - 从未上报 token 的会话（通用 ACP / lazy seed）快照投影 `None`——webui
//!   显「未上报」，不以全零冒充「已上报 0」；
//! - 上报过的会话投影**会话累计量**：随 UsageUpdate 落账、不随 Finished
//!   清零（回合缓冲 `st.usage` 的 feishu footer 语义不动）。

use sebas_acp::claude::session::AcpEvent;
use sebas_channels::ChannelKey;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::{Mapping, SessionMap};

fn web_key(tag: &str) -> ChannelKey {
    ChannelKey::new("web", format!("usage-{tag}"))
}

fn turn_usage(model: Option<&str>, input: Option<u64>, output: Option<u64>) -> sebas_acp::TurnUsage {
    sebas_acp::TurnUsage {
        model: model.map(str::to_string),
        input_tokens: input,
        output_tokens: output,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}

async fn spawn_session(router: &DispatchHandle, key: &ChannelKey, sid: &str) {
    router
        .map
        .insert(key.clone(), Mapping::active(sid))
        .await
        .unwrap();
}

#[tokio::test]
async fn sessions_that_never_report_tokens_project_none_not_zero() {
    let (router, _out_rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("none");
    spawn_session(&router, &key, "s-none").await;
    // 非 usage 事件 lazy seed 卡态：该会话从未上报任何 token 计数。
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-none".into(),
        })
        .await;
    let info = router.session_info_for(&key).await.expect("mapped session");
    assert_eq!(info.usage, None, "unreported session must project None");
    // 全 None 的 usage 帧（lazy seed 形态）同样不算「已上报」。
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-none".into(),
            usage: turn_usage(None, None, None),
        })
        .await;
    let info = router.session_info_for(&key).await.unwrap();
    assert_eq!(info.usage, None, "all-None usage frames are not a report");
}

#[tokio::test]
async fn reported_usage_accumulates_across_turns_and_survives_finish() {
    let (router, _out_rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("total");
    spawn_session(&router, &key, "s-total").await;

    // 第一回合：in 100 / out 10 → Finished（回合缓冲清零，累计量保留）。
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-total".into(),
            usage: turn_usage(Some("claude-x"), Some(100), Some(10)),
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-total".into(),
        })
        .await;

    // 第二回合：in 150 / out 25 → 快照 = 两回合累计（不是本回合终值）。
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-total".into(),
            usage: turn_usage(Some("claude-x"), Some(150), Some(25)),
        })
        .await;
    let info = router.session_info_for(&key).await.expect("mapped session");
    let usage = info.usage.expect("reported session projects Some");
    assert_eq!(usage.total_input, 250, "session-cumulative input");
    assert_eq!(usage.total_output, 35, "session-cumulative output");
    assert_eq!(usage.model.as_deref(), Some("claude-x"));
}

// ── fix-webui-qa-round12 2.1（R12-B-1，design D2）：crash 不回退 ─────────────

/// crash（terminal Error → 退役 + drop_card）后，同 key 的恢复会话（fallback-
/// fresh 换 routing id）的首次 usage 上报以幸存的会话累计为基线续增：头部与
/// 卡态累计单调不回退（600·60 → 恢复后 700·70，而非新子进程的 100·10）。
#[tokio::test]
async fn crash_then_recovered_child_accumulates_on_top_of_the_preserved_total() {
    let (router, _out_rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("crash");
    spawn_session(&router, &key, "s-crash").await;

    // 已完成回合累计到 600·60。
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-crash".into(),
            usage: turn_usage(Some("claude-x"), Some(600), Some(60)),
        })
        .await;

    // crash：watchdog 的 terminal Error 走即时路径（退役 + drop_card）。
    router
        .apply_event_to_out(
            "s-crash".to_string(),
            &AcpEvent::Error {
                session_id: "s-crash".into(),
                message: "agent process exited or hung (watchdog)".into(),
                terminal: true,
            },
        )
        .await;

    // 幸存窗口：卡态已清、映射已退役，投影以幸存者兜底——头部不消失不回退。
    let info = router.session_info_for(&key).await.expect("retired row stays");
    let usage = info.usage.expect("survivor keeps the cumulative visible");
    assert_eq!(usage.total_input, 600, "pre-crash input preserved");
    assert_eq!(usage.total_output, 60, "pre-crash output preserved");

    // 恢复：fallback-fresh 换 routing id（同 key 新映射）。
    router
        .map
        .insert(key.clone(), Mapping::active("s-new"))
        .await
        .unwrap();

    // 恢复后的第一个回合：新子进程重报 100·10 —— 在保留值之上续增。
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-new".into(),
            usage: turn_usage(Some("claude-x"), Some(100), Some(10)),
        })
        .await;
    let info = router.session_info_for(&key).await.expect("recovered session");
    let usage = info.usage.expect("recovered session projects Some");
    assert_eq!(
        usage.total_input, 700,
        "recovered turns accumulate on top of the preserved total"
    );
    assert_eq!(usage.total_output, 70, "output likewise monotonic");

    // 卡态是唯一的后续记账基线：下一回合继续累加（幸存者已一次性并入）。
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-new".into(),
            usage: turn_usage(None, Some(10), Some(1)),
        })
        .await;
    let info = router.session_info_for(&key).await.unwrap();
    let usage = info.usage.expect("still reported");
    assert_eq!(usage.total_input, 710, "subsequent turns keep accumulating");
}

/// 显式 close 清除幸存者：关闭会话的历史用量不得复活到同 key 的未来新会话
/// （对照上一用例：resume 保留、close 清零）。
#[tokio::test]
async fn explicit_close_clears_the_survivor_for_the_key() {
    let (router, _out_rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("close");
    spawn_session(&router, &key, "s-old").await;
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-old".into(),
            usage: turn_usage(Some("claude-x"), Some(600), Some(60)),
        })
        .await;
    router
        .apply_event_to_out(
            "s-old".to_string(),
            &AcpEvent::Error {
                session_id: "s-old".into(),
                message: "crashed".into(),
                terminal: true,
            },
        )
        .await;

    // 操作者显式关闭（不走 resume）。
    router.web_close_session(key.clone()).await;

    // 同 key 的新会话从零开始：首回合上报 5·1 → 累计恰为 5·1。
    router
        .map
        .insert(key.clone(), Mapping::active("s-fresh"))
        .await
        .unwrap();
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-fresh".into(),
            usage: turn_usage(None, Some(5), Some(1)),
        })
        .await;
    let info = router.session_info_for(&key).await.expect("fresh session");
    let usage = info.usage.expect("fresh session reports its own");
    assert_eq!(
        usage.total_input, 5,
        "a closed session's usage must not resurrect on a new session"
    );
    assert_eq!(usage.total_output, 1);
}
