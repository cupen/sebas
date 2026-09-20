//! add-ws-rpc-protocol contract tests the in-module unit tests cannot
//! reach through the public API alone:
//!
//! 1. spec「codec 缝与 JSON 首实现 / 换 codec 不改帧语义」— a second codec
//!    implementation swaps in behind the [`WsCodec`] seam and every frame
//!    semantic (three shapes, id correlation, result/error mutex,
//!    tolerance matrix) is unchanged; only the wire bytes differ.
//! 2. spec「既有事件迁入 Notification」— all seven migrated events travel
//!    as `Notification{method: 原 type, params: 原 payload}` with params
//!    field names preserved verbatim. The frontend reconstitutes the event
//!    object from `params`, so a renamed field would silently break views.

use sebas_dispatch::TurnEntry;
use sebas_webui::events::{PendingSubmissionView, WebUiEvent, notification_frame};
use sebas_webui::ws_rpc::{
    DecodeError, Frame, JsonCodec, NotificationFrame, RequestFrame, ResponseFrame, WsCodec,
};
use serde_json::{Value, json};

/// A second codec implementation with a deliberately different byte
/// representation: the JSON text, character-reversed. Frame semantics ride
/// the same three structs — swapping codecs must change nothing but bytes.
struct ReverseJsonCodec;

impl WsCodec for ReverseJsonCodec {
    fn encode(&self, frame: &Frame) -> String {
        JsonCodec.encode(frame).chars().rev().collect()
    }

    fn decode(&self, raw: &str) -> Result<Frame, DecodeError> {
        let forward: String = raw.chars().rev().collect();
        JsonCodec.decode(&forward)
    }
}

fn representative_frames() -> Vec<Frame> {
    vec![
        Frame::Request(RequestFrame {
            id: 7,
            method: "ping".into(),
            params: json!({"deep": [1, 2, 3]}),
        }),
        Frame::Response(ResponseFrame::ok(7, json!("pong"))),
        Frame::Response(ResponseFrame::error(7, "unknown_method", "no such method")),
        Frame::Notification(NotificationFrame {
            method: "turn.append".into(),
            params: json!({"session_id": "oc_a", "entries": [], "seq": 4}),
        }),
    ]
}

/// spec 场景「换 codec 不改帧语义」：第二实现（ReverseJsonCodec）替换 JSON
/// 实现后，三帧模型的语义字段不变（各自的 encode→decode 无损回到同一
/// Frame），线上字节表示确实变了。
#[test]
fn swapped_codec_preserves_frame_semantics_and_changes_only_the_bytes() {
    for frame in representative_frames() {
        let json_bytes = JsonCodec.encode(&frame);
        let swapped_bytes = ReverseJsonCodec.encode(&frame);
        assert_ne!(
            json_bytes, swapped_bytes,
            "the swap must change the byte representation: {frame:?}"
        );
        assert_eq!(
            ReverseJsonCodec.decode(&swapped_bytes).unwrap(),
            frame,
            "swapped codec must roundtrip losslessly: {frame:?}"
        );
        assert_eq!(
            JsonCodec.decode(&json_bytes).unwrap(),
            frame,
            "JSON codec baseline: {frame:?}"
        );
    }
}

/// id 回显与 result/error 互斥在换 codec 后原样成立（语义在帧类型里，
/// 不在字节表示里）。
#[test]
fn swapped_codec_keeps_id_echo_and_result_error_mutex() {
    let ok = Frame::Response(ResponseFrame::ok(7, json!("pong")));
    let raw = ReverseJsonCodec.encode(&ok);
    let v: Value = serde_json::from_str(raw.chars().rev().collect::<String>().as_str()).unwrap();
    assert_eq!(v["id"], 7, "id echoes through any codec: {v}");
    assert!(v.get("result").is_some() && v.get("error").is_none(), "{v}");

    let err = Frame::Response(ResponseFrame::error(9, "unknown_method", "nope"));
    let raw = ReverseJsonCodec.encode(&err);
    let v: Value = serde_json::from_str(raw.chars().rev().collect::<String>().as_str()).unwrap();
    assert_eq!(v["id"], 9, "error id echoes through any codec: {v}");
    assert_eq!(v["error"]["code"], "unknown_method");
    assert!(
        v.get("result").is_none(),
        "error side stays free of result through any codec: {v}"
    );
}

/// 容忍矩阵跨 codec 一致：未知字段容忍、非帧形状拒绝——换 codec 不得
/// 改变前向兼容语义。
#[test]
fn swapped_codec_tolerance_matrix_matches_the_json_implementation() {
    let tolerant = r#"{"id":1,"method":"ping","future_field":true}"#;
    let expected = Frame::Request(RequestFrame {
        id: 1,
        method: "ping".into(),
        params: Value::Null,
    });
    let reversed: String = tolerant.chars().rev().collect();
    assert_eq!(JsonCodec.decode(tolerant).unwrap(), expected);
    assert_eq!(ReverseJsonCodec.decode(&reversed).unwrap(), expected);

    for raw in ["", "not json", "42", "[1, 2, 3]", "{}"] {
        let reversed: String = raw.chars().rev().collect();
        assert!(
            JsonCodec.decode(raw).is_err() && ReverseJsonCodec.decode(&reversed).is_err(),
            "both codecs must reject {raw:?}"
        );
    }
}

/// spec「既有事件迁入 Notification」：全部 7 种事件的封套形状——
/// `method` = 原 dotted type，`params` = 原载荷逐字段保真（前端 view 层
/// 从 params 重建事件对象，字段名漂移会静默破坏视图）。既有
/// `events_serialize_with_dotted_type_tag` 钉住裸帧形状，这里证明封套
/// 只是搬运：params == 裸 JSON 剥掉 type。
#[test]
fn all_seven_events_travel_as_notifications_with_params_verbatim() {
    // （session-parallel-liveness-and-unread-polish 2.1）session.created /
    // updated 为相位帧（phase flatten 进 params）；round5 6.3 起载荷扩展
    // `label`（操作者命名随帧下发）。
    let phase = sebas_webui::events::SessionPhaseFrame {
        status_slug: "working".into(),
        turn_engaged: true,
        msg_count: 2,
        pending: Vec::new(),
        label: Some("renamed".into()),
    };
    let events: Vec<WebUiEvent> = vec![
        WebUiEvent::SessionCreated {
            session_id: "oc_a".into(),
            phase: sebas_webui::events::SessionPhaseFrame {
                status_slug: "starting".into(),
                turn_engaged: true,
                msg_count: 0,
                pending: Vec::new(),
                label: None,
            },
        },
        WebUiEvent::SessionUpdated {
            session_id: "oc_a".into(),
            phase,
        },
        WebUiEvent::SessionRemoved {
            session_id: "oc_b".into(),
        },
        WebUiEvent::SessionPendingDropped {
            session_id: "oc_b".into(),
            dropped: vec![PendingSubmissionView {
                id: 5,
                text: "never ran".into(),
                disposition: sebas_dispatch::PendingDisposition::Turn,
                priority: true,
            }],
        },
        WebUiEvent::ConfigUpdated,
        WebUiEvent::PermissionRequested {
            request_id: "toolu_01ABC".into(),
            session_id: "oc_a".into(),
            tool_name: "bash".into(),
            args: json!({"command": "rm -rf build"}),
            reason: "may modify state".into(),
        },
        WebUiEvent::TurnAppend {
            session_id: "web%00web-1".into(),
            entries: vec![TurnEntry::markdown(3, "hello ")],
            seq: 3,
        },
    ];

    for event in events {
        let bare = serde_json::to_value(&event).unwrap();
        let method = bare["type"].as_str().expect("dotted type tag").to_string();
        let mut params = bare.clone();
        params.as_object_mut().unwrap().remove("type");

        let Frame::Notification(notification) = notification_frame(event.clone()) else {
            panic!("{method} must wrap into a Notification");
        };
        assert_eq!(notification.method, method, "method = 原 dotted type");
        assert_eq!(
            notification.params, params,
            "params must be the bare payload verbatim for {method}"
        );
        assert!(
            notification.params.get("type").is_none(),
            "type tag must not linger in params: {method}"
        );
    }
}

/// 契约边界抽查（view 层消费的具体字段名）：permission.requested 的
/// `request_id`、turn.append 的 `entries`/`seq`、session.pending_dropped
/// 的 `dropped` 在 params 里与裸帧逐字段同形。
#[test]
fn envelope_params_keep_the_view_consumed_field_names() {
    let Frame::Notification(n) = notification_frame(WebUiEvent::PermissionRequested {
        request_id: "toolu_01ABC".into(),
        session_id: "oc_enc%00key".into(),
        tool_name: "bash".into(),
        args: json!({"command": "rm -rf build"}),
        reason: "may modify state".into(),
    }) else {
        panic!("permission.requested must be a Notification");
    };
    assert_eq!(n.method, "permission.requested");
    assert_eq!(n.params["request_id"], "toolu_01ABC");
    assert_eq!(n.params["session_id"], "oc_enc%00key");
    assert_eq!(n.params["tool_name"], "bash");
    assert_eq!(n.params["args"]["command"], "rm -rf build");
    assert_eq!(n.params["reason"], "may modify state");

    let Frame::Notification(n) = notification_frame(WebUiEvent::TurnAppend {
        session_id: "web%00web-1".into(),
        entries: vec![TurnEntry::markdown(3, "hello ")],
        seq: 3,
    }) else {
        panic!("turn.append must be a Notification");
    };
    assert_eq!(n.method, "turn.append");
    assert_eq!(n.params["session_id"], "web%00web-1");
    assert_eq!(n.params["seq"], 3);
    assert_eq!(n.params["entries"][0]["content"], "hello ");

    let Frame::Notification(n) = notification_frame(WebUiEvent::SessionPendingDropped {
        session_id: "oc_b".into(),
        dropped: vec![PendingSubmissionView {
            id: 5,
            text: "never ran".into(),
            disposition: sebas_dispatch::PendingDisposition::Turn,
            priority: true,
        }],
    }) else {
        panic!("session.pending_dropped must be a Notification");
    };
    assert_eq!(n.method, "session.pending_dropped");
    assert_eq!(n.params["dropped"][0]["id"], 5);
    assert_eq!(n.params["dropped"][0]["text"], "never ran");
    assert_eq!(n.params["dropped"][0]["disposition"], "turn");
    assert_eq!(n.params["dropped"][0]["priority"], true);
}
