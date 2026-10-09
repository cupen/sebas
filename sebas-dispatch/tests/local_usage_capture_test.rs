//! 本地落账钩子的引擎级单测（add-local-usage-statistics 2.3）。
//!
//! 合同（local-usage-capture「ACP turn usage is persisted locally」）：
//! - 每个 ACP 回合结算**恰好一行**（UsageUpdate 逐帧累计 → Finished/Error
//!   结算；Error+Finished 配对不双计）；
//! - 未上报 token 的回合仍产一行（只计请求数），token 逐字段 `None`
//!   （不以全零冒充「已上报 0」）；
//! - 钩子未装配（im-only router / 单测）时结算静默零动作。

use sebas_acp::claude::session::AcpEvent;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use sebas_domain::usage::UsageRecord;
use std::sync::{Arc, Mutex};

fn turn_usage(model: Option<&str>, input: Option<u64>, output: Option<u64>) -> sebas_acp::TurnUsage {
    sebas_acp::TurnUsage {
        model: model.map(str::to_string),
        input_tokens: input,
        output_tokens: output,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}

/// 装配带捕获钩子的 router + 一个已 seed 的回合。
async fn harness(
    tag: &str,
) -> (
    DispatchHandle,
    Arc<Mutex<Vec<UsageRecord>>>,
    mpsc::Receiver<sebas_dispatch::Out>,
) {
    let (router, out_rx) = DispatchHandle::new(SessionMap::new());
    let captured: Arc<Mutex<Vec<UsageRecord>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = captured.clone();
    router.set_usage_recorder(Arc::new(move |rec| {
        sink.lock().unwrap().push(rec);
    }));
    // seed_card = 回合开轮单点（spawn 首轮与 emit_turn_card 都经它）。
    router.seed_card(format!("s-{tag}"), format!("prompt-{tag}")).await;
    (router, captured, out_rx)
}

/// 两帧 UsageUpdate（含 model）+ Finished → 恰好一行，四类 token 为累计值。
#[tokio::test]
async fn usage_updates_settle_into_exactly_one_local_row() {
    let (router, captured, _rx) = harness("settle").await;
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-settle".into(),
            usage: turn_usage(Some("claude-sonnet"), Some(10), Some(50)),
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-settle".into(),
            usage: turn_usage(None, Some(1), None),
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-settle".into(),
        })
        .await;

    let rows = captured.lock().unwrap();
    assert_eq!(rows.len(), 1, "逐回合恰好一行，got {}", rows.len());
    let rec = &rows[0];
    assert_eq!(rec.model.as_deref(), Some("claude-sonnet"));
    assert_eq!(rec.input_tokens, Some(11), "两帧累计");
    assert_eq!(rec.output_tokens, Some(50));
    assert_eq!(rec.status, 200, "Finished = 完成");
    assert_eq!(rec.protocol, "acp");
    assert_eq!(rec.key, "", "key 恒空（无 per-key 身份）");
    // ts 是完成时刻（共享形状；RFC3339 由域层构造单点保证）。
    assert!(!rec.ts.is_empty(), "ts={}", rec.ts);
}

/// 收尾型 Error + Finished 配对（pump 拒绝路径）：恰好一行、状态 500。
#[tokio::test]
async fn error_finished_pair_lands_one_failure_row() {
    let (router, captured, _rx) = harness("pair").await;
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-pair".into(),
            usage: turn_usage(Some("m"), Some(3), Some(4)),
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s-pair".into(),
            message: "refused".into(),
            terminal: false,
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-pair".into(),
        })
        .await;

    let rows = captured.lock().unwrap();
    assert_eq!(rows.len(), 1, "配对帧只产一行，got {}", rows.len());
    assert_eq!(rows[0].status, 500);
    assert_eq!(rows[0].error.as_deref(), Some("refused"));
    assert_eq!(rows[0].input_tokens, Some(3));
}

/// 「模式未变」契约错误不是回合收尾：不结算、不影响本回合累计——随后的
/// Finished 正常产一行（200）。
#[tokio::test]
async fn mode_unchanged_contract_errors_do_not_settle() {
    let (router, captured, _rx) = harness("mode").await;
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-mode".into(),
            usage: turn_usage(Some("m"), Some(7), None),
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s-mode".into(),
            message: format!("mode unchanged ({})", sebas_dispatch::engine::MODE_UNCHANGED_MARKER),
            terminal: false,
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-mode".into(),
        })
        .await;
    let rows = captured.lock().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, 200, "契约错误不伪造失败回合");
    assert_eq!(rows[0].input_tokens, Some(7));
}

/// 从未上报 token 的回合仍产一行（只计请求数），token 全 None 不冒充零。
#[tokio::test]
async fn unreported_turn_still_counts_a_request_with_null_tokens() {
    let (router, captured, _rx) = harness("unreported").await;
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-unreported".into(),
        })
        .await;
    let rows = captured.lock().unwrap();
    assert_eq!(rows.len(), 1, "回合有行（请求数由聚合读出）");
    assert_eq!(rows[0].input_tokens, None);
    assert_eq!(rows[0].output_tokens, None);
    assert_eq!(rows[0].cache_read_tokens, None);
    assert_eq!(rows[0].cache_creation_tokens, None);
    assert_eq!(rows[0].status, 200);
}

/// 无开轮记录的 Finished（lazy seed / 占位幽灵回合）不产行——回合从未开过。
#[tokio::test]
async fn finished_without_a_turn_open_lands_no_row() {
    let (router, captured, _rx) = harness("lazy").await;
    // 第二个从未 seed 的会话直接 Finished。
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-ghost".into(),
        })
        .await;
    // s-lazy 只有一条（来自 harness 的 seed）；s-ghost 零行。
    let rows = captured.lock().unwrap();
    assert_eq!(rows.len(), 0, "幽灵回合不产行");
    drop(rows);
    // s-lazy 自己 Finished → 一行。
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-lazy".into(),
        })
        .await;
    assert_eq!(captured.lock().unwrap().len(), 1);
}

/// 钩子未装配：结算静默零动作（绝不 panic、绝不影响事件流）。
#[tokio::test]
async fn missing_recorder_settles_silently() {
    let (router, _out_rx) = DispatchHandle::new(SessionMap::new());
    router.seed_card("s-quiet".into(), "go".into()).await;
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-quiet".into(),
            usage: turn_usage(Some("m"), Some(1), Some(1)),
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-quiet".into(),
        })
        .await;
    // 未 panic 即通过（行无人接收 = 丢弃）。
}

/// 【真实驱动顺序回归，add-local-usage-statistics 6.1 沙箱发现】
/// claude 驱动的 result 帧映射顺序曾是 **Finished 先、UsageUpdate 后**，
/// 结算发生在引擎 Finished 臂——累计器随收尾清零，usage 落进孤儿条目被
/// 下一轮 seed 覆盖，行丢 token/model（沙箱证据：芯片 100/10 的回合落行
/// `acp|NULL|200|NULL|NULL|0`）。driver 已改为 usage 先行（sebas-acp
/// 成功分支 `[UsageUpdate, Finished]`），本用例按**修复后的真实顺序**
/// 驱动引擎，钉住「行携带该回合上报的 token」（spec「ACP turn lands one
/// local row」）不再回归。
#[tokio::test]
async fn real_driver_order_finished_before_usage_still_lands_reported_tokens() {
    let (router, captured, _rx) = harness("realorder").await;
    // 真实顺序：UsageUpdate 先到（进累计器），Finished 随后（结算落行）。
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "s-realorder".into(),
            usage: turn_usage(Some("claude-sonnet"), Some(100), Some(10)),
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-realorder".into(),
        })
        .await;

    let rows = captured.lock().unwrap();
    assert_eq!(rows.len(), 1, "逐回合恰好一行");
    assert_eq!(
        rows[0].input_tokens,
        Some(100),
        "行必须携带本回合上报的 input（spec「ACP turn lands one local row」）"
    );
    assert_eq!(rows[0].output_tokens, Some(10));
}

use tokio::sync::mpsc;
