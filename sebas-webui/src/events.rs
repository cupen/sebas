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

/// （session-parallel-liveness-and-unread-polish 2.1，design D2）会话相位帧
/// 的载荷：`session.updated` 与 `session.created` 同形，**五键每次帧必带**——
/// 旧 `status` 人读字段删除，无 serde 缺省、无「只在 true 时上 wire」的兼容保留
/// （core/webui/frontend 同 binary 发布，wire 无独立版本号）。前端所有「这个
/// 会话当前状态」的路径只消费这一份帧事实，不做字符串回退。
///
/// （fix-webui-qa-defects-round5 6.3）`label` 随帧扩展（proposal Non-goals 的
/// 「既有帧载荷扩展，非新帧型」）：操作者命名随每次帧如实下发——rail 据此把
/// 帧触发的行名重取收窄为「帧 label 与行已知 label 不一致才调度」，无关相位
/// 帧（状态/队列翻转）不再放大请求。
#[derive(Debug, Clone, Serialize)]
pub struct SessionPhaseFrame {
    /// 七词相位 `starting|queued|working|done|failed|waiting|dormant`
    /// （`SessionStatus::derive` 对 (MappingState, phase, 泊车) 的投影）。
    pub status_slug: String,
    /// 回合占用（WORKING ∨ 泊车 ∨ spawn 窗口）的引擎事实，总是携带。
    pub turn_engaged: bool,
    /// 可见回复段数（rail 徽标数据源），总是携带。
    pub msg_count: u64,
    /// 待生效提交全量（投递序，原石 `PendingSubmission`），总是携带。
    pub pending: Vec<sebas_dispatch::PendingSubmission>,
    /// 操作者命名（5.1，design D6；None = 未设置，行名回退预览/短 id）。
    pub label: Option<String>,
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
    /// A new session was created.（2.1，D2）与 updated 同形的完整相位帧，
    /// 初值 = Spawning 占位（starting / turn_engaged=true / 0 段 / 空栈）。
    #[serde(rename = "session.created")]
    SessionCreated {
        session_id: String,
        #[serde(flatten)]
        phase: SessionPhaseFrame,
    },
    /// A session's state was updated.（2.1，D2）五键齐全的相位事实帧。
    #[serde(rename = "session.updated")]
    SessionUpdated {
        session_id: String,
        #[serde(flatten)]
        phase: SessionPhaseFrame,
    },
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
    ///
    /// （session-parallel-liveness-and-unread-polish 2.1，design D2）
    /// `session.created` / `session.updated` 是相位帧——
    /// `status_slug`/`turn_engaged`/`msg_count`/`pending` 每帧必带，旧
    /// `status` 字段不再出现（无兼容保留）；round5 6.3 起载荷扩展 `label`
    /// （操作者命名随帧下发，rail 据此收窄行名重取）。
    #[test]
    fn events_serialize_with_dotted_type_tag() {
        let phase = crate::events::SessionPhaseFrame {
            status_slug: "working".into(),
            turn_engaged: true,
            msg_count: 3,
            pending: vec![sebas_dispatch::PendingSubmission {
                id: 9,
                text: "queued behind the live turn".into(),
                position: 0,
                disposition: sebas_dispatch::PendingDisposition::Turn,
                priority: false,
            }],
            label: Some("renamed-by-operator".into()),
        };
        let cases: Vec<(WebUiEvent, serde_json::Value)> = vec![
            (
                WebUiEvent::SessionCreated {
                    session_id: "oc_a".into(),
                    phase: crate::events::SessionPhaseFrame {
                        status_slug: "starting".into(),
                        turn_engaged: true,
                        msg_count: 0,
                        pending: Vec::new(),
                        label: None,
                    },
                },
                json!({
                    "type": "session.created",
                    "session_id": "oc_a",
                    "status_slug": "starting",
                    "turn_engaged": true,
                    "msg_count": 0,
                    "pending": [],
                    "label": null
                }),
            ),
            (
                WebUiEvent::SessionUpdated {
                    session_id: "oc_a".into(),
                    phase,
                },
                json!({
                    "type": "session.updated",
                    "session_id": "oc_a",
                    "status_slug": "working",
                    "turn_engaged": true,
                    "msg_count": 3,
                    "pending": [
                        {"id": 9, "text": "queued behind the live turn", "position": 0,
                         "disposition": "turn", "priority": false}
                    ],
                    "label": "renamed-by-operator"
                }),
            ),
            // 非占用相位同样键齐全：turn_engaged=false 显式上 wire（D2：
            // 不再「只在 true 时上 wire」——键缺省即旧世界的回退分支）。
            (
                WebUiEvent::SessionUpdated {
                    session_id: "oc_idle".into(),
                    phase: crate::events::SessionPhaseFrame {
                        status_slug: "dormant".into(),
                        turn_engaged: false,
                        msg_count: 0,
                        pending: Vec::new(),
                        label: None,
                    },
                },
                json!({
                    "type": "session.updated",
                    "session_id": "oc_idle",
                    "status_slug": "dormant",
                    "turn_engaged": false,
                    "msg_count": 0,
                    "pending": [],
                    "label": null
                }),
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
