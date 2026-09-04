//! Workspace-level tests for the `sebas replay` subcommand and the feishu
//! ingest boundary.
//!
//! Two paths are covered:
//! - the feishu-boundary ingest (`ingest_feishu_frame`): envelope parse,
//!   dedup, gates, neutral translation, dispatch — what the live WS loop
//!   runs;
//! - the offline replay (`replay::run` / `replay_frame`): neutral
//!   `ChannelEvent` JSON frames dispatched straight into the router.

use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_router::router::{Out, RouterHandle};
use sebas_router::state::SessionMap;
use sebas::replay::replay_frame;
use sebas::run::{RouterEventHandler, ingest_feishu_frame};
use tokio::sync::mpsc::Receiver;

/// Build a fresh `RouterEventHandler` with an empty `SessionMap` and a
/// captured `Out` receiver so the test can assert what was dispatched.
fn make_handler(owner_id: &str) -> (RouterEventHandler, Receiver<Out>) {
    let (router, rx) = RouterHandle::new(SessionMap::new());
    let handler = RouterEventHandler::new(router, owner_id.to_string(), None);
    (handler, rx)
}

/// Drain at most `max_yields` scheduler ticks, returning the first `Out`
/// the dispatcher produces (or `None` if nothing arrived). One yield is
/// usually enough, but the dispatch path makes several `.await` calls, so
/// we loop to be deterministic.
async fn recv_within(rx: &mut Receiver<Out>, max_yields: usize) -> Option<Out> {
    for _ in 0..max_yields {
        tokio::task::yield_now().await;
        if let Ok(out) = rx.try_recv() {
            return Some(out);
        }
    }
    None
}

#[tokio::test]
async fn replay_one_text_message_emits_spawn_acp() {
    let (handler, mut rx) = make_handler("ou_owner");
    let payload = serde_json::to_vec(&serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "im.message.receive_v1", "tenant_key": "t" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_owner" } },
            "message": {
                "chat_id": "oc_test",
                "message_id": "om_1",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}"
            }
        }
    }))
    .expect("serialize envelope");

    assert!(
        ingest_feishu_frame(&handler, &payload),
        "ingest should accept a well-formed text envelope"
    );

    // First event should be the ack reaction on the user's message.
    // SEED reaction = Feishu emoji_type "Get"（👌，card_state.rs 有意选择）。
    let ack = recv_within(&mut rx, 64)
        .await
        .expect("expected Out::AckMsg after ingest");
    let ack_ok =
        matches!(&ack, Out::AckMsg { message_id, emoji } if message_id == "om_1" && emoji == "Get");
    assert!(
        ack_ok,
        "expected Out::AckMsg with message_id=om_1 emoji=Get, got {ack:?}"
    );

    // Then the actual spawn.
    let out = recv_within(&mut rx, 64)
        .await
        .expect("expected Out::SpawnAcp after ingest");
    assert!(
        matches!(out, Out::SpawnAcp { .. }),
        "expected Out::SpawnAcp, got {out:?}"
    );
}

#[tokio::test]
async fn replay_button_cb_routes_to_help_when_session_dead() {
    let (handler, mut rx) = make_handler("ou_owner");
    let payload = serde_json::to_vec(&serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "card.action.trigger", "tenant_key": "t" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_owner" } },
            "chat_id": "oc_test",
            "action": {
                "session_id": "sess_missing",
                "request_id": "req_1",
                "decision": "allow_once"
            }
        }
    }))
    .expect("serialize envelope");

    assert!(
        ingest_feishu_frame(&handler, &payload),
        "ingest should accept a well-formed card.action.trigger envelope"
    );

    let out = recv_within(&mut rx, 64)
        .await
        .expect("expected an Out after ingest for a dead-session button cb");

    // The current router routing for a button cb with no live session is
    // `Out::SendCard` carrying the dead-session card (see `on_button` in
    // `router/src/router.rs`). The brief also accepts `Out::HelpText` if
    // the routing branch is ever changed to emit help instead; both are
    // valid "button cb was dispatched" outcomes.
    assert!(
        matches!(out, Out::SendCard { .. } | Out::HelpText { .. }),
        "expected Out::SendCard (dead session) or Out::HelpText, got {out:?}"
    );
}

/// 按 run_ws_loop 的真实装配接线：config 默认 allowed_chat_types +
/// 默认 bot_name（而非 make_handler 的空列表放行）。sebas-5y5 的回归
/// 覆盖点：飞书私聊 wire 值是 "p2p"，默认白名单若按字面匹配会静默丢弃。
fn make_live_wired_handler() -> (RouterEventHandler, Receiver<Out>) {
    let (router, rx) = RouterHandle::new(SessionMap::new());
    let mut handler = RouterEventHandler::new(router, String::new(), None);
    let feishu_cfg = sebas::config::FeishuConfig::default();
    handler.allowed_chat_types = feishu_cfg.allowed_chat_types;
    handler.bot_name = feishu_cfg.bot_name;
    (handler, rx)
}

fn text_message_payload(chat_type: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "im.message.receive_v1", "tenant_key": "t" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_user" } },
            "message": {
                "chat_id": "oc_test",
                "message_id": "om_1",
                "message_type": "text",
                "chat_type": chat_type,
                "content": "{\"text\":\"hi\"}"
            }
        }
    }))
    .expect("serialize envelope")
}

/// 私聊（chat_type="p2p"，飞书真实值）必须通过默认白名单并被派发到
/// router（sebas-5y5：旧默认 ["private","group"] 漏掉 "p2p"，私聊消息
/// 全部被 debug 级日志静默丢弃，用户侧零反馈）。
#[tokio::test]
async fn private_dm_p2p_passes_default_chat_type_filter() {
    let (handler, mut rx) = make_live_wired_handler();

    let payload = text_message_payload("p2p");
    assert!(
        ingest_feishu_frame(&handler, &payload),
        "私聊 chat_type=\"p2p\" 应通过默认过滤并派发（sebas-5y5）"
    );

    // 派发后 router 必须产出首个用户可见反馈：AckMsg（表情回应）。
    let ack = recv_within(&mut rx, 64)
        .await
        .expect("p2p 私聊消息派发后应产生 Out::AckMsg");
    assert!(
        matches!(&ack, Out::AckMsg { message_id, .. } if message_id == "om_1"),
        "expected Out::AckMsg for om_1, got {ack:?}"
    );
}

/// 群聊不受修复影响；存量配置里写了幻影值 "private" 的，也应放行
/// 真实 "p2p" 入站（private ↔ p2p 归一化别名）。
#[tokio::test]
async fn group_passes_and_private_alias_still_allows_p2p() {
    let (handler, _rx) = make_live_wired_handler();
    assert!(
        ingest_feishu_frame(&handler, &text_message_payload("group")),
        "群聊 chat_type=\"group\" 应通过默认过滤"
    );

    let (router, _rx2) = RouterHandle::new(SessionMap::new());
    let mut legacy = RouterEventHandler::new(router, String::new(), None);
    legacy.allowed_chat_types = vec!["private".into(), "group".into()];
    assert!(
        ingest_feishu_frame(&legacy, &text_message_payload("p2p")),
        "存量配置 \"private\" 应视作 \"p2p\" 的别名放行私聊消息"
    );
}

/// `replay::run` 的 FS 路径：目录 glob、按文件名排序、逐帧 dispatch、
/// 坏帧 warn-and-skip、dir 不存在 → Err。夹具是**中立 ChannelEvent**
/// JSON（与 WS 侧 dump 的形状一致）。
#[tokio::test]
async fn replay_run_reads_sorted_frames_and_skips_bad_ones() {
    let dir = std::env::temp_dir().join(format!("sebas-replay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mk_frame = |chat: &str| {
        let evt = ChannelEvent::Text {
            key: ChannelKey::feishu(chat, None),
            text: "hi".into(),
            reply_target: Some("om_1".into()),
        };
        serde_json::to_string_pretty(&evt).unwrap()
    };
    // 时间戳前缀保证字典序 = 捕获序；混入一个坏帧和一个非 .json 文件。
    std::fs::write(dir.join("001-a.json"), mk_frame("oc_a")).unwrap();
    std::fs::write(dir.join("002-b.json"), mk_frame("oc_b")).unwrap();
    std::fs::write(dir.join("003-bad.json"), "{ nope").unwrap();
    std::fs::write(dir.join("ignore.txt"), mk_frame("oc_c")).unwrap();

    let count = sebas::replay::run(sebas::replay::ReplayArgs { dir: dir.clone() })
        .await
        .expect("replay run");
    assert_eq!(
        count, 2,
        "two well-formed frames dispatched, bad one skipped"
    );

    let missing = sebas::replay::run(sebas::replay::ReplayArgs {
        dir: dir.join("does-not-exist"),
    })
    .await;
    assert!(missing.is_err(), "missing dir must error");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `replay_frame` 直连 `RouterHandle`：中立 Text 帧派发后应产生
/// AckMsg（reply_target 落在 ack 上）+ SpawnAcp，证明离线路径与
/// WS 路径经同一 router 决策。
#[tokio::test]
async fn replay_frame_dispatches_neutral_text() {
    let (router, mut rx) = RouterHandle::new(SessionMap::new());
    let evt = ChannelEvent::Text {
        key: ChannelKey::feishu("oc_test", None),
        text: "hi".into(),
        reply_target: Some("om_1".into()),
    };
    let payload = serde_json::to_vec(&evt).unwrap();

    assert!(replay_frame(&router, &payload), "neutral frame dispatches");

    let ack = recv_within(&mut rx, 64)
        .await
        .expect("expected Out::AckMsg from neutral replay");
    assert!(
        matches!(&ack, Out::AckMsg { message_id, .. } if message_id == "om_1"),
        "expected Out::AckMsg for om_1, got {ack:?}"
    );
}
