//! 端到端节流：fake-claude "stream" prompt 连发 5 个 TextDelta + end_turn。
//! 经 spawn_acp_pump（production 路径：acp_spawn_and_activate → seed_card
//! 隐含于 pump 的 lazy seed，但此处显式走 dispatch_out 不便，故直接驱动 pump）
//! 断言 150ms 内合并成 1 个含 5 段的 UpdateCard，随后 Finished 立即产 ✅ 卡。

use acp_claude::manager::SessionManager;
use feishu::events::{FeishuIn, SessionKey};
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/fake-claude")
}

/// Stringify the card payload carried by an `Out` variant so assertions can
/// string-search the rendered content. `Out` itself only derives `Debug`
/// (router/src/router.rs:15), not `Serialize`, so we drill into the inner
/// `card: serde_json::Value` — same pattern as `tests/pump_unit_test.rs`.
fn card_str(out: &Out) -> String {
    match out {
        Out::UpdateCard { card, .. } | Out::SendCard { card, .. } => {
            serde_json::to_string(card).unwrap()
        }
        other => panic!("expected card-bearing Out, got {other:?}"),
    }
}

#[tokio::test]
async fn fake_claude_stream_merges_five_chunks_then_done() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(30)));
    let key = SessionKey {
        chat_id: "oc_stream".into(),
        thread_id: None,
    };

    // Text "stream" -> SpawnAcp.
    router
        .dispatch(FeishuIn::Text {
            key: key.clone(),
            text: "stream".into(),
            reply_to: None,
        })
        .await;
    let out = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Out::SpawnAcp { key: k, prompt } = out else {
        panic!("expected SpawnAcp, got {out:?}")
    };

    // 走 production spawn：create_session + rx 克隆 + CreateSession prompt + activate.
    let (session_id, _pending, rx) = sebas::run::acp_spawn_and_activate(
        &mgr,
        &router,
        &k,
        &prompt,
        fake().to_str().unwrap(),
        vec![],
        None,
    )
    .await
    .expect("spawn ok");

    // 显式 seed_card（production 在 dispatch_out 里调，此处 pump 单测路径补上）。
    router.seed_card(session_id.clone(), prompt.clone()).await;

    // 跑 production pump。
    sebas::run::spawn_acp_pump(rx, router.clone(), session_id.clone());

    // 第一个 UpdateCard：含 5 个 chunk0..chunk4，emoji 🚧。
    let first = tokio::time::timeout(Duration::from_millis(600), out_rx.recv())
        .await
        .expect("first merged UpdateCard within 600ms")
        .expect("channel open");
    let s = card_str(&first);
    for i in 0..5 {
        assert!(
            s.contains(&format!("chunk{i}")),
            "chunk{i} in merged card: {s}"
        );
    }
    assert!(s.contains("🚧"));

    // Finished 立即产 ✅ 卡。p3g 起 Out 序列里还混有 reaction（tick 的 🚧、
    // Finished 自带的 ✅），按类别找：✅ 卡 + ✅ reaction 都必须出现。
    let mut got_done = false;
    let mut got_done_react = false;
    for _ in 0..4 {
        let o = tokio::time::timeout(Duration::from_millis(400), out_rx.recv())
            .await
            .expect("recv in time")
            .expect("channel open");
        match o {
            Out::UpdateCard { .. } | Out::SendCard { .. } => {
                if card_str(&o).contains("✅") {
                    got_done = true;
                }
            }
            Out::React { ref emoji, .. } if emoji == "✅" => got_done_react = true,
            Out::React { .. } => {} // tick 补发的 🚧，忽略
            other => panic!("unexpected out: {other:?}"),
        }
        if got_done && got_done_react {
            break;
        }
    }
    assert!(got_done, "Finished 必产 ✅ 卡");
    assert!(got_done_react, "Finished 必换 ✅ reaction");
}
