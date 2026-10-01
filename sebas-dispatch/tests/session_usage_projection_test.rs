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
