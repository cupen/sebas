//! End-to-end: ACP 权限请求经 `InProcessBackend` 往返到 webui 审查卡。
//!
//! 验证 design D6 补上的权限半场：router 的 ACP 权限广播 → InProcessBackend
//! 转发为 `PermissionNotice`（session_id = URL-safe encoded key），
//! `answer_permission` 把决策映射回 `AcpCommand::PermissionReply` 并经
//! `Out::SendAcp` 回路由（`Escalate` 降级 `AllowOnce`）。

use sebas_acp::claude::session::{AcpCommand, AcpEvent, Decision};
use sebas_channels::ChannelKey;
use sebas_router::router::Out;
use sebas_router::state::{Mapping, SessionMap};
use sebas_webui::session_backend::{
    InProcessBackend, PermissionDecision, SessionBackend,
};
use std::sync::Arc;
use std::time::Duration;

fn key(chat_id: &str) -> ChannelKey {
    ChannelKey::feishu(chat_id, None)
}

/// 与 routes::encode_session_key 同形的 URL-safe 编码（channel\0reference）。
fn encoded_key(k: &ChannelKey) -> String {
    urlencoding::encode(&format!("{}\0{}", k.channel.as_str(), k.reference))
        .into_owned()
}

#[tokio::test]
async fn acp_permission_round_trips_through_in_process_backend() {
    let map = SessionMap::new();
    let key = key("oc_perm");
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = sebas_router::RouterHandle::new(map);
    let backend = InProcessBackend::new(router.clone());

    // 先订阅审查卡流，再触发权限请求（broadcast 只转发订阅后的事件）。
    let mut notices = backend
        .permission_requests()
        .expect("in-process backend has permission notices");

    router
        .dispatch_acp_event(AcpEvent::PermissionRequest {
            session_id: "s1".into(),
            request_id: "claude:toolu_1".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"cmd": "ls"}),
        })
        .await;

    // 审查卡流里拿到 PermissionNotice，session_id 是 URL-safe encoded key。
    let notice = tokio::time::timeout(Duration::from_secs(5), notices.recv())
        .await
        .expect("notice timeout")
        .expect("notice");
    assert_eq!(notice.request_id, "claude:toolu_1");
    assert_eq!(notice.session_id, encoded_key(&key), "session_id 必须是 URL-safe encoded key");
    assert_eq!(notice.tool_name, "Bash");
    assert_eq!(notice.args, serde_json::json!({"cmd": "ls"}));

    // 回填 allow-once：后端把决策映射回 PermissionReply，经 Out::SendAcp 回路。
    assert!(
        backend
            .answer_permission("claude:toolu_1", PermissionDecision::AllowOnce)
            .await,
        "answer must find the pending request"
    );

    let reply = drain_permission_reply(&mut out_rx).await;
    match reply {
        Out::SendAcp {
            session_id,
            cmd:
                AcpCommand::PermissionReply {
                    session_id: sid,
                    request_id,
                    decision,
                },
        } => {
            assert_eq!(session_id, "s1");
            assert_eq!(sid, "s1");
            assert_eq!(request_id, "claude:toolu_1");
            assert!(matches!(decision, Decision::AllowOnce));
        }
        other => panic!("expected SendAcp PermissionReply, got {other:?}"),
    }
}

#[tokio::test]
async fn escalate_decision_downgrades_to_allow_once() {
    let map = SessionMap::new();
    let key = key("oc_esc");
    map.insert(key.clone(), Mapping::active("s2"))
        .await
        .unwrap();
    let (router, mut out_rx) = sebas_router::RouterHandle::new(map);
    let backend = InProcessBackend::new(router.clone());

    let mut notices = backend.permission_requests().expect("notices");
    router
        .dispatch_acp_event(AcpEvent::PermissionRequest {
            session_id: "s2".into(),
            request_id: "acp:gemini:tool_9".into(),
            tool_name: "Edit".into(),
            args: serde_json::json!({"path": "a.txt"}),
        })
        .await;
    let notice = tokio::time::timeout(Duration::from_secs(5), notices.recv())
        .await
        .expect("notice timeout")
        .expect("notice");
    assert_eq!(notice.request_id, "acp:gemini:tool_9");

    // ACP 无 escalate 等价：降级为 AllowOnce（design D6/R5）。
    assert!(
        backend
            .answer_permission(
                "acp:gemini:tool_9",
                PermissionDecision::Escalate {
                    reason: "need network once".into()
                }
            )
            .await
    );
    match drain_permission_reply(&mut out_rx).await {
        Out::SendAcp {
            cmd: AcpCommand::PermissionReply { decision, .. },
            ..
        } => assert!(matches!(decision, Decision::AllowOnce), "escalate 必须降级为 AllowOnce"),
        other => panic!("expected SendAcp PermissionReply, got {other:?}"),
    }
}

#[tokio::test]
async fn unknown_request_answers_false() {
    let (router, _out_rx) = sebas_router::RouterHandle::new(SessionMap::new());
    let backend = InProcessBackend::new(router);
    assert!(
        !backend
            .answer_permission("nope", PermissionDecision::Deny)
            .await,
        "unknown request id must answer false"
    );
}

/// 排掉权限卡（SendCard）与可能的其它 Out，直到取到 PermissionReply。
async fn drain_permission_reply(
    out_rx: &mut tokio::sync::mpsc::Receiver<Out>,
) -> Out {
    loop {
        let got = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
            .await
            .expect("permission reply not received in time")
            .expect("channel closed");
        if matches!(got, Out::SendAcp { .. }) {
            return got;
        }
    }
}

// 供编译期确认后端被 as trait object 使用时仍满足 SessionBackend。
#[allow(dead_code)]
fn _assert_trait_object(b: Arc<dyn SessionBackend>) {
    let _ = b;
}

