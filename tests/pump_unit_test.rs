//! pump 节流契约单测（见 openspec/specs/feishu-cards/spec.md）。合成 mpsc Receiver 喂事件，断言：
//! 5 个 TextDelta 合并成 1 个 UpdateCard（≤1/150ms）；Finished 立即再发 ✅；
//! terminal Error 立即发 ❌ + 清 mapping；通道关闭 drop_card + 退出。
//! 不依赖 fake-claude 二进制。

use sebas::run::spawn_acp_pump;
use sebas_acp::claude::session::AcpEvent;
use sebas_dispatch::engine::{DispatchHandle, Out};
use sebas_dispatch::state::{Mapping, SessionMap};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Stringify the card payload carried by an `Out` variant so assertions can
/// string-search the rendered content. `Out` itself is not `Serialize`, but
/// each card-bearing variant carries `card: serde_json::Value` — this mirrors
/// the established pattern in `router/tests/terminal_error_test.rs`.
fn card_str(out: &Out) -> String {
    match out {
        Out::UpdateCard { card, .. } | Out::SendCard { card, .. } => {
            serde_json::to_string(card).unwrap()
        }
        other => panic!("expected card-bearing Out, got {other:?}"),
    }
}

#[tokio::test]
async fn five_deltas_merge_into_one_updatecard() {
    let map = SessionMap::new();
    let (router, mut out_rx) = DispatchHandle::new(map);
    router.seed_card("s1".into(), "hi".into()).await;
    let (tx, rx) = mpsc::channel::<AcpEvent>(64);
    let rx = Arc::new(tokio::sync::Mutex::new(rx));
    spawn_acp_pump(rx, router.clone(), "s1".into());

    for i in 0..5 {
        tx.send(AcpEvent::TextDelta {
            session_id: "s1".into(),
            delta: format!("chunk{i} "),
        })
        .await
        .unwrap();
    }
    let first = tokio::time::timeout(Duration::from_millis(400), out_rx.recv())
        .await
        .expect("first UpdateCard within 400ms")
        .expect("channel open");
    let s = card_str(&first);
    for i in 0..5 {
        assert!(s.contains(&format!("chunk{i}")), "chunk{i} in card: {s}");
    }
    // p3g：stater emoji 不再进卡标题（标题为主题），工作态由紧跟的
    // reaction 表达 —— 见下方 second 的 WORKING 断言。
    let second = tokio::time::timeout(Duration::from_millis(120), out_rx.recv())
        .await
        .expect("React 🚧 follows the merged card")
        .expect("channel open");
    assert!(
        matches!(second, Out::React { ref emoji, .. } if emoji == sebas_dispatch::card_state::phase::WORKING),
        "合并卡后紧跟 React WORKING: {second:?}"
    );
    let third = tokio::time::timeout(Duration::from_millis(120), out_rx.recv()).await;
    assert!(third.is_err(), "150ms 窗口内不得再有第三个 Out");
}

#[tokio::test]
async fn finished_flushes_immediately_after_stream() {
    let map = SessionMap::new();
    let (router, mut out_rx) = DispatchHandle::new(map);
    router.seed_card("s2".into(), "p".into()).await;
    let (tx, rx) = mpsc::channel::<AcpEvent>(64);
    let rx = Arc::new(tokio::sync::Mutex::new(rx));
    spawn_acp_pump(rx, router.clone(), "s2".into());

    tx.send(AcpEvent::TextDelta {
        session_id: "s2".into(),
        delta: "x".into(),
    })
    .await
    .unwrap();
    tx.send(AcpEvent::Finished {
        session_id: "s2".into(),
    })
    .await
    .unwrap();

    let mut got_done = false;
    for _ in 0..3 {
        let o = tokio::time::timeout(Duration::from_millis(300), out_rx.recv())
            .await
            .expect("recv in time")
            .expect("channel open");
        // 状态由 Feishu reaction 表达，不再进卡：DONE 反应即 Finished 落地。
        match o {
            Out::React { ref emoji, .. } if emoji == sebas_dispatch::card_state::phase::DONE => {
                got_done = true;
                break;
            }
            Out::UpdateCard { .. } | Out::SendCard { .. } | Out::React { .. } => continue,
            other => panic!("unexpected out: {other:?}"),
        }
    }
    assert!(got_done, "Finished 必产 DONE reaction");
}

#[tokio::test]
async fn terminal_error_flushes_removes_and_exits() {
    let map = SessionMap::new();
    let key = sebas_channels::ChannelKey::feishu("oc_t", None);
    map.insert(key.clone(), Mapping::active("s3"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    router.seed_card("s3".into(), "p".into()).await;
    let (tx, rx) = mpsc::channel::<AcpEvent>(64);
    let rx = Arc::new(tokio::sync::Mutex::new(rx));
    spawn_acp_pump(rx, router.clone(), "s3".into());

    tx.send(AcpEvent::TextDelta {
        session_id: "s3".into(),
        delta: "before".into(),
    })
    .await
    .unwrap();
    tx.send(AcpEvent::Error {
        session_id: "s3".into(),
        message: "crashed".into(),
        terminal: true,
    })
    .await
    .unwrap();

    let mut got_red = false;
    for _ in 0..3 {
        let o = tokio::time::timeout(Duration::from_millis(300), out_rx.recv())
            .await
            .expect("recv in time")
            .expect("channel open");
        let s = card_str(&o);
        if s.contains("❌") && s.contains("before") && s.contains("crashed") {
            got_red = true;
            break;
        }
    }
    assert!(
        got_red,
        "terminal 必产含 ❌ + 死前 transcript + 错误正文的卡"
    );
    // （fix-webui-qa-defects-round4 1.2）terminal 拆除只退役活跃绑定：映射
    // 以 Dormant 记录形态保留（会话行不消失、转录可回看），不再是 Active。
    let m = map.get(&key).await.expect("记录（Dormant）必须保留");
    assert!(m.session_id().is_none(), "活跃绑定必须清掉: {m:?}");
}

// ── fix-webui-qa-round7 2.2（acp-model-selection D2）：模型拒绝即时收尾 ──────

/// 带稳定标记的非终态 Error（模型切换拒绝）必须走即时路径：SEED 占位不被
/// 推成 WORKING 滞留，而是经 apply_event_to_out 锚定收尾 DONE——拒绝即终态，
/// 远早于 600s watchdog（QA DEF-02 的 pump 半边）。
#[tokio::test]
async fn model_rejection_error_is_immediate_in_the_pump() {
    let map = SessionMap::new();
    let key = sebas_channels::ChannelKey::feishu("oc_round7", None);
    map.insert(key.clone(), Mapping::active("s-model"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map);
    // 接收回执相位（SEED + prompt）——拒绝到达时的典型窗口。
    router.seed_card("s-model".into(), "p".into()).await;
    let (tx, rx) = mpsc::channel::<AcpEvent>(64);
    let rx = Arc::new(tokio::sync::Mutex::new(rx));
    spawn_acp_pump(rx, router.clone(), "s-model".into());

    tx.send(AcpEvent::Error {
        session_id: "s-model".into(),
        message: format!(
            "set model \"bad-model\" 被拒绝（Invalid params），{}",
            sebas_acp::MODEL_UNCHANGED_MARKER
        ),
        terminal: false,
    })
    .await
    .unwrap();

    // 即时性：不推进 150ms 节流窗，收尾产物（flush 的 UpdateCard）即刻到达。
    let out = tokio::time::timeout(Duration::from_millis(100), out_rx.recv())
        .await
        .expect("model rejection settles immediately (no debounce)")
        .expect("channel open");
    assert!(matches!(out, Out::UpdateCard { .. }), "got {out:?}");

    // 相位终态：DONE（非 WORKING 滞留）；拒绝条目可见。
    let phase = router
        .card_state_snapshot()
        .await
        .remove("s-model")
        .map(|st| st.status_emoji);
    assert_eq!(
        phase.as_deref(),
        Some(sebas_dispatch::card_state::phase::DONE),
        "拒绝即终态收尾"
    );
    let turns = router.session_turns(&key, 0).await.unwrap();
    assert!(
        turns
            .iter()
            .any(|e| e.content.contains("bad-model") && e.content.contains("模型未变")),
        "拒绝回执如实落转录"
    );
}

/// 对照组：不带标记的普通非终态 Error（claude refusal 配对的前半）仍走流式
/// 臂——FSM 照旧 SEED→WORKING，**不**触发即时收尾；终态语义留给配对的
/// Finished，配对语义不受影响。（不设时序断言：pump 的 interval 首 tick 与
/// 事件到达存在良性竞态，flush 时刻不定；相位结果是确定的。）
#[tokio::test]
async fn unmarked_nonterminal_error_keeps_the_streaming_arm() {
    let map = SessionMap::new();
    let (router, mut out_rx) = DispatchHandle::new(map);
    router.seed_card("s-pair".into(), "p".into()).await;
    let (tx, rx) = mpsc::channel::<AcpEvent>(64);
    let rx = Arc::new(tokio::sync::Mutex::new(rx));
    spawn_acp_pump(rx, router.clone(), "s-pair".into());

    tx.send(AcpEvent::Error {
        session_id: "s-pair".into(),
        message: "agent declined the request".into(),
        terminal: false,
    })
    .await
    .unwrap();

    // 等流式臂的 debounce flush 落地（≤150ms 窗 + 余量）。
    let deadline = std::time::Instant::now() + Duration::from_millis(600);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), out_rx.recv()).await {
            Ok(Some(_)) => break,
            Ok(None) => panic!("channel closed"),
            Err(_) => {}
        }
    }
    let phase = router
        .card_state_snapshot()
        .await
        .remove("s-pair")
        .map(|st| st.status_emoji);
    assert_eq!(
        phase.as_deref(),
        Some(sebas_dispatch::card_state::phase::WORKING),
        "无标记的非终态 Error 走流式臂（SEED→WORKING），收尾留给配对 Finished——\
         与带标记的模型拒绝（即时 DONE）语义分野"
    );
}
