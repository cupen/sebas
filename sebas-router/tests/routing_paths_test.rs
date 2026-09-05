//! 路由分支补测（覆盖率目标 router/ ≥ 90% 的补齐测试）：on_button 死会话/
//! 缺 rid/未知 decision，slash 转发臂，Media 组合 prompt，dispatch_acp_event
//! 全事件类型，MsgIdMap 存取，terminal error 清理。

use sebas_acp::claude::session::{AcpCommand, AcpEvent, Decision};
use sebas_channels::{ChannelAction, ChannelEvent, ChannelKey};
use sebas_router::router::{Out, RouterHandle, compose_media_prompt};
use sebas_router::state::{Mapping, SessionMap};
use std::time::Duration;

fn key() -> ChannelKey {
    ChannelKey::feishu("oc_x", None)
}

fn text(key: ChannelKey, text: impl Into<String>, reply_target: Option<&str>) -> ChannelEvent {
    ChannelEvent::Text {
        key,
        text: text.into(),
        reply_target: reply_target.map(str::to_string),
    }
}

async fn next_out(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Out {
    tokio::time::timeout(Duration::from_millis(300), rx.recv())
        .await
        .expect("out within 300ms")
        .expect("channel open")
}

/// 收干一小段时间窗内的全部 Out（p3g 起 FSM 转移会连带 Out::React，
/// 不能再假设一个事件只产一个 Out）。
async fn drain(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Vec<Out> {
    let mut out = vec![];
    while let Ok(Some(o)) = tokio::time::timeout(Duration::from_millis(60), rx.recv()).await {
        out.push(o);
    }
    out
}

// ---------- ReplyTargetMap 生命周期（F2 修复） ----------

/// spawn_new 必须清掉该 key 的 reply target：新会话不继承上一条入站的
/// 回复目标（话题内 root_id），否则 ReplyTargetMap 随话题数无界增长。
#[tokio::test]
async fn spawn_new_clears_reply_target() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_topic", Some("omt_t1"));
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());

    // 入站话题消息写入 reply target；已映射会话走 Continue，不触发 spawn_new。
    router
        .dispatch(text(key.clone(), "hello", Some("om_root")))
        .await;
    assert_eq!(router.reply_target(&key).await.as_deref(), Some("om_root"));

    // 摘掉映射后下一条消息走 SpawnNew → spawn_new 必须清 reply target。
    map.remove_by_key(&key).await;
    router
        .dispatch(text(key.clone(), "fresh", Some("om_root")))
        .await;
    assert_eq!(
        router.reply_target(&key).await,
        None,
        "spawn_new must clear the stale reply target"
    );

    let _ = drain(&mut out_rx).await;
}

// ---------- on_button 分支 ----------

#[tokio::test]
async fn button_cb_dead_session_gets_dead_card() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(ChannelEvent::ButtonCb { key: key(), action: ChannelAction {
                session_id: "ghost".into(),
                request_id: Some("r1".into()),
                decision: Some("allow_once".into()),
                value: serde_json::Value::Null,
            }, })
        .await;
    match next_out(&mut out_rx).await {
        Out::SendCard { card, .. } => {
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("已结束") || s.contains("无效"), "dead card: {s}");
        }
        other => panic!("expected SendCard, got {other:?}"),
    }
}

#[tokio::test]
async fn button_cb_missing_request_id_gets_help() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(ChannelEvent::ButtonCb { key: key(), action: ChannelAction {
                session_id: "s1".into(),
                request_id: None,
                decision: Some("allow_once".into()),
                value: serde_json::Value::Null,
            }, })
        .await;
    assert!(matches!(next_out(&mut out_rx).await, Out::HelpText { .. }));
}

#[tokio::test]
async fn button_cb_unknown_decision_fails_closed_to_deny() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);
    // In-place flip (commit 658b312) requires a pre-recorded perm_card entry;
    // see tests/permission_flow_test.rs:128 for the recipe.
    router
        .record_perm_card_msg_id(
            "r9".into(),
            key(),
            "om_fake".into(),
            "Bash".into(),
            serde_json::json!({"cmd": "yolo"}),
        )
        .await;
    router
        .dispatch(ChannelEvent::ButtonCb { key: key(), action: ChannelAction {
                session_id: "s1".into(),
                request_id: Some("r9".into()),
                decision: Some("yolo".into()),
                value: serde_json::Value::Null,
            }, })
        .await;
    // First Out is the in-place flip (UpdateCardByMsgId); drain until SendAcp.
    let out = loop {
        let got = next_out(&mut out_rx).await;
        if matches!(got, Out::SendAcp { .. }) {
            break got;
        }
    };
    match out {
        Out::SendAcp { cmd, .. } => match cmd {
            AcpCommand::PermissionReply {
                decision,
                request_id,
                ..
            } => {
                assert!(matches!(decision, Decision::Deny));
                assert_eq!(request_id, "r9");
            }
            other => panic!("expected PermissionReply, got {other:?}"),
        },
        other => panic!("expected SendAcp, got {other:?}"),
    }
}

// ---------- slash 转发臂 ----------

#[tokio::test]
async fn slash_compact_cost_cancel_forward_to_live_session() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);

    for (cmd_text, expect) in [("/compact", "/compact"), ("/cost", "/cost")] {
        router
            .dispatch(text(key(), cmd_text, None))
            .await;

        if cmd_text == "/compact" {
            // /compact now sends a progress card first, then the command
            match next_out(&mut out_rx).await {
                Out::SendCard { .. } => {} // progress card
                other => panic!("expected SendCard for /compact, got {other:?}"),
            }
        }

        match next_out(&mut out_rx).await {
            Out::SendAcp {
                cmd: AcpCommand::ContinueSession { session_id, prompt },
                ..
            } => {
                assert_eq!(session_id, "s1");
                assert_eq!(prompt, expect);
            }
            other => panic!("expected ContinueSession for {cmd_text}, got {other:?}"),
        }
    }

    router
        .dispatch(text(key(), "/cancel", None))
        .await;
    match next_out(&mut out_rx).await {
        Out::SendAcp {
            cmd: AcpCommand::Cancel { session_id },
            ..
        } => assert_eq!(session_id, "s1"),
        other => panic!("expected Cancel, got {other:?}"),
    }
}

#[tokio::test]
async fn slash_compact_without_session_gets_plain_error() {
    // sebas-ixv：无会话时 /compact 不再静默（HelpText 在 dispatch 层是
    // no-op），改为 PlainText 明确报错。
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(text(key(), "/compact", None))
        .await;
    match next_out(&mut out_rx).await {
        Out::PlainText { content, .. } => {
            assert!(content.contains("没有活跃会话"), "content: {content}");
            assert!(content.contains("/compact"), "content: {content}");
        }
        other => panic!("expected PlainText, got {other:?}"),
    }
}

#[tokio::test]
async fn slash_status_cancel_without_session_get_plain_error() {
    // sebas-ixv：/status /cancel 无会话同样明确报错（与 /compact 同约定）。
    for cmd_text in ["/status", "/cancel"] {
        let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
        router
            .dispatch(text(key(), cmd_text, None))
            .await;
        match next_out(&mut out_rx).await {
            Out::PlainText { content, .. } => {
                assert!(content.contains("没有活跃会话"), "{cmd_text}: {content}");
                assert!(content.contains(cmd_text), "{cmd_text}: {content}");
            }
            other => panic!("expected PlainText for {cmd_text}, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn switch_resume_cd_get_unsupported_reply() {
    // sebas-ixv：/switch /resume /cd 已解析但路由未接入，必须明确回复
    // 「暂未支持」，不得静默丢弃。
    for cmd_text in ["/switch 1", "/resume s1", "/cd /tmp"] {
        let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
        router
            .dispatch(text(key(), cmd_text, None))
            .await;
        match next_out(&mut out_rx).await {
            Out::PlainText { content, .. } => {
                let cmd = cmd_text.split_whitespace().next().unwrap();
                assert!(content.contains("暂未支持"), "{cmd_text}: {content}");
                assert!(content.contains(cmd), "{cmd_text}: {content}");
            }
            other => panic!("expected PlainText for {cmd_text}, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn slash_status_forwards_continue_session() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(text(key(), "/status", None))
        .await;

    match next_out(&mut out_rx).await {
        Out::SendAcp {
            cmd: AcpCommand::ContinueSession { session_id, prompt },
            ..
        } => {
            assert_eq!(session_id, "s1");
            assert_eq!(prompt, "/status");
        }
        other => panic!("expected ContinueSession(\"/status\"), got {other:?}"),
    }
}

#[tokio::test]
async fn slash_sessions_lists_empty() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(text(key(), "/sessions", None))
        .await;

    match next_out(&mut out_rx).await {
        Out::PlainText { content, .. } => {
            assert!(content.contains("没有活跃会话"), "content: {content}");
        }
        other => panic!("expected PlainText, got {other:?}"),
    }
}

#[tokio::test]
async fn slash_sessions_lists_active() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(text(key(), "/sessions", None))
        .await;

    match next_out(&mut out_rx).await {
        Out::PlainText { content, .. } => {
            assert!(content.contains("s1"), "s1 missing: {content}");
            assert!(content.contains("active"), "label missing: {content}");
        }
        other => panic!("expected PlainText, got {other:?}"),
    }
}

// ---------- Media 组合 ----------

#[tokio::test]
async fn media_message_composes_prompt_and_spawns() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(ChannelEvent::Media { key: key(), files: vec!["/tmp/a.png".into(), "/tmp/b.pdf".into()], caption: Some("看这两张".into()), reply_target: None })
        .await;
    match next_out(&mut out_rx).await {
        Out::SpawnAcp { prompt, .. } => {
            assert!(prompt.contains("看这两张"), "caption in prompt: {prompt}");
            assert!(
                prompt.contains("[attached: /tmp/a.png, /tmp/b.pdf]"),
                "files in prompt: {prompt}"
            );
        }
        other => panic!("expected SpawnAcp, got {other:?}"),
    }
}

#[test]
fn compose_media_prompt_handles_empty_caption() {
    let p = compose_media_prompt("", &["/x".to_string()]);
    assert_eq!(p, "\n[attached: /x]");
    let p2 = compose_media_prompt("cap", &[]);
    assert!(p2.starts_with("cap\n"));
}

// ---------- dispatch_acp_event 全类型 ----------

#[tokio::test]
async fn dispatch_acp_event_routes_every_variant() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    router.seed_card("s1".into(), "p".into()).await;

    let events = vec![
        AcpEvent::TextDelta {
            session_id: "s1".into(),
            delta: "d".into(),
        },
        AcpEvent::ThinkingDelta {
            session_id: "s1".into(),
            delta: "t".into(),
        },
        AcpEvent::ToolStart {
            session_id: "s1".into(),
            tool_name: "Bash".into(),
            args: serde_json::Value::Null,
        },
        AcpEvent::ToolProgress {
            session_id: "s1".into(),
            tool_name: "Bash".into(),
            progress: "p".into(),
        },
        AcpEvent::ToolEnd {
            session_id: "s1".into(),
            tool_name: "Bash".into(),
            result: "r".into(),
        },
        AcpEvent::Error {
            session_id: "s1".into(),
            message: "warn".into(),
            terminal: false,
        },
    ];
    for evt in events {
        router.dispatch_acp_event(evt).await;
        let outs = drain(&mut out_rx).await;
        assert!(
            outs.iter().any(|o| matches!(o, Out::UpdateCard { .. })),
            "expected UpdateCard in {outs:?}"
        );
    }
}

#[tokio::test]
async fn dispatch_acp_event_permission_request_sends_card() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch_acp_event(AcpEvent::PermissionRequest {
            session_id: "s1".into(),
            request_id: "r1".into(),
            tool_name: "Write".into(),
            args: serde_json::json!({}),
        })
        .await;
    assert!(matches!(next_out(&mut out_rx).await, Out::SendCard { .. }));
}

#[tokio::test]
async fn permission_request_without_key_is_dropped() {
    // session 无对应 SessionKey 时权限卡丢弃（warn 日志路径）。
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch_acp_event(AcpEvent::PermissionRequest {
            session_id: "ghost".into(),
            request_id: "r1".into(),
            tool_name: "Write".into(),
            args: serde_json::json!({}),
        })
        .await;
    let r = tokio::time::timeout(Duration::from_millis(150), out_rx.recv()).await;
    assert!(r.is_err(), "no Out may be emitted for unknown session");
}

#[tokio::test]
async fn terminal_error_removes_mapping_and_drops_card() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    router.seed_card("s1".into(), "p".into()).await;
    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s1".into(),
            message: "boom".into(),
            terminal: true,
        })
        .await;
    let out = next_out(&mut out_rx).await;
    assert!(matches!(out, Out::UpdateCard { .. }));
    // terminal Error 不再发 Out::React 换 FAILED（❌ 行已 push 到 body）；
    // queue timeout 之后应无更多 Out。
    assert!(map.get(&key()).await.is_none(), "mapping removed");
    // CardState 已清：后续 flush 是 no-op。
    router.flush_card("s1").await;
    let r = tokio::time::timeout(Duration::from_millis(150), out_rx.recv()).await;
    assert!(r.is_err());
}

// ---------- MsgIdMap / insert_mapping / fail_spawn 杂项 ----------

#[tokio::test]
async fn root_msg_id_round_trip_via_handle() {
    let map = SessionMap::new();
    let (router, _rx) = RouterHandle::new(map);
    assert!(router.root_msg_id("s1").await.is_none());
    router.record_root_msg_id("s1".into(), "om_1".into()).await;
    assert_eq!(router.root_msg_id("s1").await.as_deref(), Some("om_1"));
}

#[tokio::test]
async fn insert_mapping_marks_alive_and_routes() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    assert!(!router.session_alive(&key()).await);
    router.insert_mapping(key(), "s7".into()).await;
    assert!(router.session_alive(&key()).await);
    router
        .dispatch(text(key(), "yo", None))
        .await;
    // Per-turn flow: a fresh card is posted first, then the prompt is
    // forwarded to the session.
    match next_out(&mut out_rx).await {
        Out::SendCard { .. } => {}
        other => panic!("expected per-turn SendCard, got {other:?}"),
    }
    match next_out(&mut out_rx).await {
        Out::SendAcp { session_id, .. } => assert_eq!(session_id, "s7"),
        other => panic!("expected SendAcp, got {other:?}"),
    }
}

#[tokio::test]
async fn fail_spawn_ignores_active_and_missing_entries() {
    let map = SessionMap::new();
    // 无条目：no-op 不 panic。
    map.fail_spawn(&key()).await;
    // Active 条目：不动。
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    map.fail_spawn(&key()).await;
    assert_eq!(map.get(&key()).await.unwrap().session_id(), Some("s1"));
    // 不存在 session 的 lookup：None。
    assert!(map.lookup_key_by_session("ghost").await.is_none());
    // remove_by_session 对不存在的 session：no-op。
    map.remove_by_session("ghost").await;
    // activate 无占位 → 插入新映射（warn 路径）。
    let orphan = ChannelKey::feishu("oc_orphan", None);
    let pending = map.activate(&orphan, "s9".into(), None, None).await;
    assert!(pending.is_empty());
    assert_eq!(map.get(&orphan).await.unwrap().session_id(), Some("s9"));
}

#[tokio::test]
async fn help_command_emits_help_card() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(text(key(), "/help", None))
        .await;
    let out = next_out(&mut out_rx).await;
    match out {
        Out::SendCard { msg_id, card, .. } => {
            // 应携带帮助卡片哨兵 tag
            assert_eq!(
                msg_id.as_deref(),
                Some("__help_card__"),
                "help card should carry HELP_CARD_TAG"
            );
            // 卡片应包含分组 tab 按钮和命令
            let s = card.to_json();
            assert!(s.contains("💬 会话管理"), "help card missing session tab");
            assert!(s.contains("/new"), "help card missing /new command");
            assert!(s.contains("/cancel"), "help card missing /cancel command");
        }
        other => panic!("expected SendCard (help card), got {other:?}"),
    }
}

// ---------- 原生执行体桥（make-feishu-optional-webui-primary） ----------

use sebas_router::native_bridge::{NativeApprovalDecision, NativeSessionBridge};
use std::sync::Arc;

/// 记录「是否走桥」的假桥：is_native 恒返回 `default_native`，prompt 记一笔
/// 并注册 mapping（模拟真实桥把会话登记进 router）。
#[derive(Clone, Default)]
struct FakeNativeBridge {
    prompted: Arc<std::sync::Mutex<Vec<String>>>,
    default_native: bool,
}

impl FakeNativeBridge {
    fn new(default_native: bool) -> Self {
        Self {
            prompted: Arc::new(std::sync::Mutex::new(Vec::new())),
            default_native,
        }
    }
    fn prompts(&self) -> Vec<String> {
        self.prompted.lock().unwrap().clone()
    }
}

impl NativeSessionBridge for FakeNativeBridge {
    fn is_native(&self, _key: &ChannelKey) -> bool {
        // 已登记的原生会话（此处简化：default_native 即判定）。
        self.default_native
    }
    fn prompt(self: Arc<Self>, key: ChannelKey, text: String) {
        self.prompted.lock().unwrap().push(format!("{}|{}", key.reference, text));
    }
    fn answer_permission(&self, _rid: &str, _d: NativeApprovalDecision) -> bool {
        false
    }
}

/// 无桥（native=None）时 feishu PassThrough 应保持现状：route_text → SpawnNew
/// → Out::SpawnAcp，不产生任何原生会话。
#[tokio::test]
async fn feishu_text_without_bridge_stays_acp() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_no_bridge", None);
    let (router, mut out_rx) = RouterHandle::new(map);

    router
        .dispatch(text(key.clone(), "hi", None))
        .await;

    let out = next_out(&mut out_rx).await;
    assert!(
        matches!(out, Out::SpawnAcp { .. }),
        "无桥时应发射 SpawnAcp（acp 桥默认），got {out:?}"
    );
}

/// default_native=true 时：新 feishu 文本经桥直达原生内核，不发射 SpawnAcp，
/// 也不渲染飞书卡片（Out 流里不应出现 SendCard/AckMsg 之外的卡片类事件）。
#[tokio::test]
async fn feishu_text_with_native_default_routes_to_bridge() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_native", None);
    let (router, mut out_rx) = RouterHandle::new(map);
    let fake = Arc::new(FakeNativeBridge::new(true));
    let bridge: Arc<dyn NativeSessionBridge> = fake.clone();
    router.set_native_bridge(Some(bridge)).await;

    router
        .dispatch(text(key.clone(), "build it", None))
        .await;

    // 走桥：prompt 收到该 chat + 消息。
    assert_eq!(fake.prompts(), vec!["oc_native|build it".to_string()]);
    // 不应发射 SpawnAcp（acp 路径被跳过）。
    let outs = drain(&mut out_rx).await;
    assert!(
        !outs.iter().any(|o| matches!(o, Out::SpawnAcp { .. })),
        "原生路径不应发射 SpawnAcp，got {outs:?}"
    );
}

/// 原生会话续聊：同一 chat 再次来消息仍走桥（default 仍为 true），不重新 spawn。
#[tokio::test]
async fn feishu_native_session_continues_via_bridge() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_native2", None);
    let (router, mut out_rx) = RouterHandle::new(map);
    let fake = Arc::new(FakeNativeBridge::new(true));
    let bridge: Arc<dyn NativeSessionBridge> = fake.clone();
    router.set_native_bridge(Some(bridge)).await;

    for cmd_text in ["first", "second"] {
        router
            .dispatch(text(key.clone(), cmd_text, None))
            .await;
    }
    assert_eq!(fake.prompts(), vec!["oc_native2|first".to_string(), "oc_native2|second".to_string()]);
    let _ = drain(&mut out_rx).await;
}
