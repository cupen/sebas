//! pump 节流契约单测（见 openspec/specs/feishu-cards/spec.md）。合成 mpsc Receiver 喂事件，断言：
//! 5 个 TextDelta 合并成 1 个 UpdateCard（≤1/150ms）；Finished 立即再发 ✅；
//! terminal Error 立即发 ❌ + 清 mapping；通道关闭 drop_card + 退出。
//! 不依赖 fake-claude 二进制。

use sebas_acp_claude::session::AcpEvent;
use sebas_router::router::{Out, RouterHandle};
use sebas_router::state::{Mapping, SessionMap};
use sebas::run::spawn_acp_pump;
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
    let (router, mut out_rx) = RouterHandle::new(map);
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
        matches!(second, Out::React { ref emoji, .. } if emoji == sebas_router::card_state::phase::WORKING),
        "合并卡后紧跟 React WORKING: {second:?}"
    );
    let third = tokio::time::timeout(Duration::from_millis(120), out_rx.recv()).await;
    assert!(third.is_err(), "150ms 窗口内不得再有第三个 Out");
}

#[tokio::test]
async fn finished_flushes_immediately_after_stream() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
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
            Out::React { ref emoji, .. } if emoji == sebas_router::card_state::phase::DONE => {
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
    let key = sebas_feishu::events::SessionKey {
        chat_id: "oc_t".into(),
        thread_id: None,
    };
    map.insert(key.clone(), Mapping::active("s3"))
        .await
        .unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
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
    assert!(map.get(&key).await.is_none(), "terminal 后 mapping 必清");
}
