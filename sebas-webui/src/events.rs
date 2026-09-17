//! Real-time events pushed to WebUI clients over the WebSocket channel.

use sebas_dispatch::{PendingDisposition, TurnEntry};
use serde::Serialize;

/// （workbench-turn-queue 7.3）一次性「未执行」提示的条目形状：id + 文本 +
/// 处置 + 优先标记。position 不随提示下发（提示只点名哪些提交没有执行）。
#[derive(Debug, Clone, Serialize)]
pub struct PendingSubmissionView {
    pub id: u64,
    pub text: String,
    pub disposition: PendingDisposition,
    pub priority: bool,
}

/// Events that the WebUI can push to connected clients.
///
/// Each event serializes to a JSON object with a `type` tag. On the wire
/// events no longer travel as bare frames: [`notification_frame`] wraps the
/// serialized shape into the WS RPC `Notification` envelope (add-ws-rpc-
/// protocol), with `method` = the dotted `type` and `params` = the payload
/// below. Names remain dotted (`session.created`), a carry-over from the
/// former SSE two-part `event: update` encoding.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum WebUiEvent {
    /// A new session was created.
    #[serde(rename = "session.created")]
    SessionCreated { session_id: String },
    /// A session's state was updated.
    #[serde(rename = "session.updated")]
    SessionUpdated { session_id: String, status: String },
    /// A session was removed.
    #[serde(rename = "session.removed")]
    SessionRemoved { session_id: String },
    /// （workbench-turn-queue 5.2/7.3）会话终结时未执行的待生效提交，逐条
    /// 列出（id/文本/处置/优先）。在 session.removed 帧之前到达。
    #[serde(rename = "session.pending_dropped")]
    SessionPendingDropped {
        session_id: String,
        dropped: Vec<PendingSubmissionView>,
    },
    /// （fix-pending-queue-liveness 2.2）某会话的停滞回合被看门狗强制收尾。
    /// `released` = 解除卡死的待执行提交条数；前端据此弹 warn 分级通知。
    #[serde(rename = "session.turn_stalled")]
    SessionTurnStalled { session_id: String, released: usize },
    /// （fix-webui-streaming-liveness 5.1，D6）重新同步信号：服务端检测到
    /// 订阅落后（WS broadcast Lagged）或 core 通道重连给出快照重取信号时
    /// 发给浏览器。无载荷——前端清本地增量游标与流式缓冲后全量重取受影响
    /// 视图，不因陈旧游标永久拒收增量。
    #[serde(rename = "session.resync")]
    SessionResync,
    /// Configuration was updated. No sender exists yet; the variant is
    /// reserved so clients must tolerate it (and unknown types) arriving.
    #[serde(rename = "config.updated")]
    ConfigUpdated,
    /// A gated tool call awaits an operator decision (the review card).
    /// `args` carries the call's arguments verbatim; the client answers via
    /// `POST /api/permissions/{request_id}/answer`.
    #[serde(rename = "permission.requested")]
    PermissionRequested {
        request_id: String,
        session_id: String,
        tool_name: String,
        args: serde_json::Value,
        reason: String,
    },
    /// 实时回合内容（workbench-live-conversation-flow 2.1）：同一合并窗内
    /// 某会话追加的 transcript 条目（落库序、position 单调）。`seq` 是本帧
    /// 最后一条的 position（前端的去重锚）。纯增量补充：乱序/迟到/丢失由
    /// 客户端以快照重取收敛，不参与重放。
    #[serde(rename = "turn.append")]
    TurnAppend {
        session_id: String,
        entries: Vec<TurnEntry>,
        seq: u64,
    },
    /// （add-core-reachability-ws-push D3/D5）核心可达性翻转推送。params 与
    /// `/api/summary` 的 `reachability` 段同形（`reachability_payload`）：
    /// 可达 `{ok:true}`；不可达 `{ok:false, kind, cause}`（kind ∈
    /// startup_failed | auth_rejected | disconnected）。前端 kind 分文案逻辑
    /// 原样复用，banner 不靠 cause 字符串匹配的既有契约不变。
    #[serde(rename = "core.reachability")]
    CoreReachability {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        kind: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cause: Option<String>,
    },
}

/// add-ws-rpc-protocol：事件统一包进 Notification 封套——`method` = 原
/// dotted `type`，`params` = 原载荷（剥掉已被 `method` 取代的 type 标签，
/// 载荷结构不变）。dotted 名字仍由上面的 serde rename 表唯一持有，这里只
/// 做搬运，不给漂移留缝。
pub fn notification_frame(event: WebUiEvent) -> crate::ws_rpc::Frame {
    let mut params = serde_json::to_value(&event).unwrap_or_default();
    let method = params
        .as_object_mut()
        .and_then(|obj| obj.remove("type"))
        .and_then(|tag| tag.as_str().map(str::to_string))
        .unwrap_or_default();
    crate::ws_rpc::Frame::Notification(crate::ws_rpc::NotificationFrame { method, params })
}

#[cfg(test)]
mod tests {

    use super::WebUiEvent;
    use sebas_dispatch::TurnEntry;
    use serde_json::json;

    /// Every event serializes to a JSON object tagged with its dotted
    /// `type`; this shape is the WS contract clients key off.
    #[test]
    fn events_serialize_with_dotted_type_tag() {
        let cases: Vec<(WebUiEvent, serde_json::Value)> = vec![
            (
                WebUiEvent::SessionCreated {
                    session_id: "oc_a".into(),
                },
                json!({"type": "session.created", "session_id": "oc_a"}),
            ),
            (
                WebUiEvent::SessionUpdated {
                    session_id: "oc_a".into(),
                    status: "active".into(),
                },
                json!({"type": "session.updated", "session_id": "oc_a", "status": "active"}),
            ),
            (
                WebUiEvent::SessionRemoved {
                    session_id: "oc_b".into(),
                },
                json!({"type": "session.removed", "session_id": "oc_b"}),
            ),
            (
                WebUiEvent::SessionPendingDropped {
                    session_id: "oc_b".into(),
                    dropped: vec![crate::events::PendingSubmissionView {
                        id: 5,
                        text: "never ran".into(),
                        disposition: sebas_dispatch::PendingDisposition::Turn,
                        priority: true,
                    }],
                },
                json!({
                    "type": "session.pending_dropped",
                    "session_id": "oc_b",
                    "dropped": [
                        {"id": 5, "text": "never ran", "disposition": "turn", "priority": true}
                    ]
                }),
            ),
            // fix-pending-queue-liveness 2.2：停滞收尾通知帧点名会话与释放数。
            (
                WebUiEvent::SessionTurnStalled {
                    session_id: "oc_b".into(),
                    released: 2,
                },
                json!({
                    "type": "session.turn_stalled",
                    "session_id": "oc_b",
                    "released": 2
                }),
            ),
            // fix-webui-streaming-liveness 5.1：resync 帧无载荷，dotted 名即契约。
            (WebUiEvent::SessionResync, json!({"type": "session.resync"})),
            (WebUiEvent::ConfigUpdated, json!({"type": "config.updated"})),
            (
                WebUiEvent::PermissionRequested {
                    request_id: "req1".into(),
                    session_id: "oc_a".into(),
                    tool_name: "bash".into(),
                    args: json!({"command": "rm -rf build"}),
                    reason: "may modify state".into(),
                },
                json!({
                    "type": "permission.requested",
                    "request_id": "req1",
                    "session_id": "oc_a",
                    "tool_name": "bash",
                    "args": {"command": "rm -rf build"},
                    "reason": "may modify state"
                }),
            ),
        ];
        for (event, want) in cases {
            let got = serde_json::to_value(&event).unwrap();
            assert_eq!(got, want, "wrong JSON shape for {want}");
        }
    }

    /// workbench-live-conversation-flow 2.1：turn.append 帧带 dotted type、
    /// encoded session id 与条目数组；seq = 最后一条的 position。
    #[test]
    fn turn_append_serializes_with_dotted_type_tag() {
        let event = WebUiEvent::TurnAppend {
            session_id: "web%00web-1".into(),
            entries: vec![
                TurnEntry::markdown(3, "hello "),
                TurnEntry::markdown(4, "world"),
            ],
            seq: 4,
        };
        let got = serde_json::to_value(&event).unwrap();
        assert_eq!(got["type"], "turn.append");
        assert_eq!(got["session_id"], "web%00web-1");
        assert_eq!(got["seq"], 4);
        assert_eq!(got["entries"].as_array().unwrap().len(), 2);
        assert_eq!(got["entries"][0]["content"], "hello ");
    }

    /// add-ws-rpc-protocol：事件以 Notification 封套投递——method = 原
    /// dotted type，params = 原载荷且不再携带 type 标签。spec 场景
    /// 「turn.append 以 Notification 到达」的帧形状钉死在这里。
    #[test]
    fn events_wrap_into_notification_envelope() {
        let frame = super::notification_frame(WebUiEvent::TurnAppend {
            session_id: "web%00web-1".into(),
            entries: vec![TurnEntry::markdown(3, "hello ")],
            seq: 3,
        });
        let crate::ws_rpc::Frame::Notification(notification) = frame else {
            panic!("events must wrap into a Notification, got {frame:?}");
        };
        assert_eq!(notification.method, "turn.append");
        assert_eq!(notification.params["session_id"], "web%00web-1");
        assert_eq!(notification.params["seq"], 3);
        assert_eq!(notification.params["entries"].as_array().unwrap().len(), 1);
        assert_eq!(notification.params["entries"][0]["content"], "hello ");
        assert!(
            notification.params.get("type").is_none(),
            "type tag must move into method, params keeps the bare payload: {}",
            notification.params
        );

        // 无载荷事件：params 退化为空对象，method 仍带 dotted 名。
        let frame = super::notification_frame(WebUiEvent::ConfigUpdated);
        let crate::ws_rpc::Frame::Notification(notification) = frame else {
            panic!("events must wrap into a Notification, got {frame:?}");
        };
        assert_eq!(notification.method, "config.updated");
        assert_eq!(notification.params, json!({}));
    }

    /// add-core-reachability-ws-push D5：可达性帧的 params 与 /api/summary
    /// 的 reachability 段同形——可达只有 `{ok:true}`（无 kind/cause 噪声），
    /// 不可达携带机器可读 kind 与原文 cause。
    #[test]
    fn core_reachability_payload_matches_summary_shape() {
        let frame = super::notification_frame(WebUiEvent::CoreReachability {
            ok: true,
            kind: None,
            cause: None,
        });
        let crate::ws_rpc::Frame::Notification(notification) = frame else {
            panic!("events must wrap into a Notification");
        };
        assert_eq!(notification.method, "core.reachability");
        assert_eq!(notification.params, json!({ "ok": true }));

        let frame = super::notification_frame(WebUiEvent::CoreReachability {
            ok: false,
            kind: Some("startup_failed"),
            cause: Some("core startup failed: bad config".into()),
        });
        let crate::ws_rpc::Frame::Notification(notification) = frame else {
            panic!("events must wrap into a Notification");
        };
        assert_eq!(notification.method, "core.reachability");
        assert_eq!(
            notification.params,
            json!({ "ok": false, "kind": "startup_failed", "cause": "core startup failed: bad config" })
        );
    }
}
