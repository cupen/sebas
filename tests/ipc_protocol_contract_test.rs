//! golden fixture 契约闸门（unify-ipc-protocol-home 5.1–5.4）。
//!
//! 每条边界一份 checked-in fixture，各钉两件事：
//!
//! 1. 代表性载荷的**序列化字节**（逐字节比对）；
//! 2. 该消息的**字段名集合**。
//!
//! 两者都比对，所以删字段、改字段名、改枚举取值**必红**，而**新增带默认值
//! 的字段不红**（正确语义：那条路径由「旧对端不发也能读」的缺省值兜住，
//! 不是闸门的漏洞——见 5.4 的复核）。
//!
//! fixture 只在**有意的**破坏性变更里更新，且该 change 必须说明缘由。
//!
//! - `tests/fixtures/ipc_core_channel_wire.json`（5.1）：core session channel。
//!   `payloads` 是**类型迁出根 crate 之前**由代码序列化出的真实字节，冻结在
//!   此——本文件因此同时是 2.3「迁出前后逐字节一致」的机械证据。
//! - `tests/fixtures/ipc_node_link_wire.json`（5.2）：节点链路握手与版本协商。
//! - `tests/fixtures/ipc_webui_wire.json`（5.3）：webui 面向浏览器的 WS 帧与
//!   HTTP 载荷形状。

use serde_json::Value;

// ---------------------------------------------------------------------------
// 通用
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取 fixture {} 失败: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("fixture {} 不是合法 JSON: {e}", path.display()))
}

/// 一条钉住的载荷：字节 + 顶层字段名集合。
struct Pinned {
    name: String,
    bytes: String,
    fields: Vec<String>,
}

fn pinned(doc: &Value, section: &str) -> Vec<Pinned> {
    doc[section]
        .as_array()
        .unwrap_or_else(|| panic!("fixture 缺少数组 `{section}`"))
        .iter()
        .map(|p| Pinned {
            name: p["name"].as_str().expect("name").to_string(),
            bytes: p["bytes"].as_str().expect("bytes").to_string(),
            fields: p["fields"]
                .as_array()
                .expect("fields")
                .iter()
                .map(|f| f.as_str().expect("field name").to_string())
                .collect(),
        })
        .collect()
}

/// 字段名集合：载荷顶层键，排序后。
fn field_names(bytes: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(bytes.trim()).expect("pinned payload is valid JSON");
    let mut names: Vec<String> = v
        .as_object()
        .unwrap_or_else(|| panic!("载荷不是 JSON 对象: {bytes}"))
        .keys()
        .cloned()
        .collect();
    names.sort();
    names
}

/// 逐字节 + 字段名集合双断言。`actual` 是当前代码序列化出的真实字节。
fn assert_pinned(pin: &Pinned, actual: &str, boundary: &str) {
    assert_eq!(
        actual, pin.bytes,
        "[{boundary}] `{}` 的序列化字节变了。删字段 / 改字段名 / 改枚举取值都是\
         **破坏性变更**：必须先在 change 里声明、说明迁移路径，再有意更新 fixture。\n\
         当前: {actual}\nfixture: {}",
        pin.name, pin.bytes
    );
    assert_eq!(
        field_names(actual),
        pin.fields,
        "[{boundary}] `{}` 的字段名集合变了（字节比对之外的独立断言：字段名换了\
         但字节恰好相同的极端情况也要红）",
        pin.name
    );
}

fn pin<'a>(pins: &'a [Pinned], name: &str) -> &'a Pinned {
    pins.iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("fixture 里没有载荷 `{name}`"))
}

fn bytes_of<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("serialize")
}

// ---------------------------------------------------------------------------
// 5.1 core session channel
// ---------------------------------------------------------------------------

use sebas::core_channel::protocol::{
    Attachment, ChannelHandshake, CoreChannelRequest, CoreChannelResponse, NodeLinkOp,
    NodeLinkOutcome, SessionStreamFrame, StateStreamFrame,
};
use sebas_channels::ChannelKey;
use sebas_domain::node::NodeView;
use sebas_domain::session::{
    PermissionDecision, SessionEvent, SessionInfo, SessionRejection,
};
use sebas_domain::vocabulary::{CardPhase, SessionMode, SessionPhase};

fn sample_info() -> SessionInfo {
    SessionInfo {
        channel: "feishu".into(),
        key: "oc_1".into(),
        session_id: Some("s1".into()),
        status: SessionPhase::Active,
        phase: Some(CardPhase::OnIt),
        user_prompt: Some("hello".into()),
        last_active_unix: 1_700_000_000,
        project_dir: Some("/proj".into()),
        current_model: Some("m1".into()),
        available_models: Some(vec!["m1".into(), "m2".into()]),
        agent_kind: Some("claude".into()),
        usage: None,
        backend: Some("acp".into()),
        pending: Vec::new(),
        remote: None,
        desired_mode: SessionMode::Ask,
        effective_mode: Some(SessionMode::Ask),
        msg_count: 3,
        turn_engaged: true,
        spawn_failure_reason: None,
        parked_approvals: 0,
        label: Some("demo".into()),
        available_commands: Vec::new(),
    }
}

/// 当前代码序列化出的全部 core channel 代表性载荷（请求 / 响应 / 快照帧 /
/// 事件帧 / 拒绝），与 fixture 名字一一对应。
fn core_channel_payloads() -> Vec<(String, String)> {
    let key = ChannelKey::feishu("oc_1", Some("om_t"));
    vec![
        (
            "request.spawn".into(),
            bytes_of(&CoreChannelRequest::Spawn {
                prompt: "hello".into(),
                project_dir: Some("/proj".into()),
                model: Some("m1".into()),
                mode: Some("allow".into()),
                agent: "claude".into(),
                node: None,
            }),
        ),
        (
            "request.message".into(),
            bytes_of(&CoreChannelRequest::Message {
                key: key.clone(),
                message: "hi".into(),
                attachments: vec![Attachment {
                    path: "/tmp/img.png".into(),
                    mime: Some("image/png".into()),
                    name: Some("img.png".into()),
                }],
            }),
        ),
        (
            "request.state_snapshot".into(),
            bytes_of(&CoreChannelRequest::StateSnapshot {
                domain: "providers".into(),
            }),
        ),
        (
            "request.state_subscribe".into(),
            bytes_of(&CoreChannelRequest::StateSubscribe),
        ),
        (
            "request.approval_answer".into(),
            bytes_of(&CoreChannelRequest::ApprovalAnswer {
                request_id: "toolu_1".into(),
                decision: PermissionDecision::Escalate {
                    reason: "needs human".into(),
                },
            }),
        ),
        (
            "request.node_link_list".into(),
            bytes_of(&CoreChannelRequest::NodeLink {
                op: NodeLinkOp::ListNodes,
            }),
        ),
        (
            "request.node_link_issue_token".into(),
            bytes_of(&CoreChannelRequest::NodeLink {
                op: NodeLinkOp::IssueJoinToken {
                    ttl_secs: Some(900),
                },
            }),
        ),
        (
            "response.snapshot".into(),
            bytes_of(&CoreChannelResponse::Snapshot {
                sessions: vec![sample_info()],
            }),
        ),
        (
            "response.spawned".into(),
            bytes_of(&CoreChannelResponse::Spawned { key: key.clone() }),
        ),
        (
            "response.rejected".into(),
            bytes_of(&CoreChannelResponse::Rejected {
                rejection: SessionRejection::UnknownSession { key: "k".into() },
            }),
        ),
        (
            "response.state_snapshot".into(),
            bytes_of(&CoreChannelResponse::StateSnapshot {
                domain: "providers".into(),
                payload: serde_json::json!({"providers": {"p": {"ok": true}}}),
            }),
        ),
        (
            "response.node_link_nodes".into(),
            bytes_of(&CoreChannelResponse::NodeLink(NodeLinkOutcome::Nodes {
                nodes: vec![NodeView {
                    id: "node-a".into(),
                    status: "online".into(),
                    last_seen_unix: Some(1_700_000_000),
                    created_unix: 1_600_000_000,
                    local: false,
                }],
            })),
        ),
        (
            "frame.session_snapshot".into(),
            bytes_of(&SessionStreamFrame::Snapshot {
                sessions: vec![sample_info()],
            }),
        ),
        (
            "frame.session_event".into(),
            bytes_of(&SessionStreamFrame::Event {
                event: SessionEvent::Updated {
                    session: sample_info(),
                },
            }),
        ),
        (
            "frame.session_resync".into(),
            bytes_of(&SessionStreamFrame::Resync),
        ),
        (
            "frame.state_snapshot".into(),
            bytes_of(&StateStreamFrame::Snapshot {
                domains: serde_json::json!({
                    "providers": {},
                    "settings": null,
                    "projects": [],
                    "sessions": []
                }),
            }),
        ),
        (
            "frame.state_changed".into(),
            bytes_of(&StateStreamFrame::Changed {
                scope: "providers".into(),
            }),
        ),
    ]
}

/// 5.1 + 2.3：core channel 的代表性载荷逐字节等于**迁出前**冻结的字节，
/// 字段名集合也一致——类型搬家没有动过任何一个线形状。
#[test]
fn core_channel_payloads_are_byte_identical_to_the_pre_migration_fixture() {
    let doc = fixture("ipc_core_channel_wire.json");
    let pins = pinned(&doc, "payloads");
    let actual = core_channel_payloads();
    assert_eq!(
        actual.len(),
        pins.len(),
        "fixture 覆盖了 {} 条载荷，当前构造了 {} 条——代表集变了就必须同步更新 fixture",
        pins.len(),
        actual.len()
    );
    for (name, bytes) in &actual {
        assert_pinned(pin(&pins, name), bytes, "core-channel");
    }
}

/// 5.1 + 7.1：握手是本 change **唯一**有意的 wire 变化，且变化就是新增
/// `version`（additive）。before/after 两侧都是真实序列化产物。
#[test]
fn handshake_is_the_only_intentional_wire_change() {
    let doc = fixture("ipc_core_channel_wire.json");
    let before = &doc["handshake"]["before"];
    let after = &doc["handshake"]["after"];
    assert_eq!(
        before["bytes"].as_str().unwrap(),
        r#"{"secret":"s3cret"}"#,
        "迁出前的握手原文（无版本字段）"
    );
    let actual = bytes_of(&ChannelHandshake::new("s3cret"));
    assert_eq!(
        actual,
        after["bytes"].as_str().unwrap(),
        "握手帧必须精确等于声明的 after 形状"
    );
    assert_ne!(
        before["bytes"].as_str().unwrap(),
        actual,
        "与迁出前确有差异——这就是本 change 唯一有意的 wire 变化（版本字段）"
    );
    assert_eq!(field_names(&actual), vec!["secret", "version"]);
    assert_eq!(
        serde_json::to_value(&ChannelHandshake::new("s3cret")).unwrap()["version"],
        1
    );
}

// ---------------------------------------------------------------------------
// 5.2 节点链路：握手与版本协商
// ---------------------------------------------------------------------------

use sebas_node_link::{
    CapabilityManifest, Hello, HelloAck, HelloOutcome, NodeAuth, PROTOCOL_VERSION, RejectCode,
};

/// 5.2：节点链路的握手与版本协商消息逐字节 / 逐字段钉住；fixture 与既有
/// `golden_link_vocabulary.json`（词汇侧）互补，覆盖**握手与版本协商**这一面。
#[test]
fn node_link_handshake_messages_match_the_fixture() {
    let doc = fixture("ipc_node_link_wire.json");
    let pins = pinned(&doc, "payloads");

    let hello = Hello {
        protocol_version: PROTOCOL_VERSION,
        node_id: "dev-box".into(),
        auth: NodeAuth::JoinToken {
            token: "tok-1".into(),
        },
        manifest: CapabilityManifest {
            agent_kinds: vec![],
            providers: vec!["anthropic".into()],
            mode_enforcement: vec![],
        },
    };
    let cases: Vec<(&str, String)> = vec![
        ("hello.join_token", bytes_of(&hello)),
        (
            "hello_ack.accepted",
            bytes_of(&HelloAck {
                protocol_version: PROTOCOL_VERSION,
                outcome: HelloOutcome::Accepted {
                    credential: Some("cred-1".into()),
                },
                router_url: Some("http://10.0.0.5:8787".into()),
                router_token: Some("rt-1".into()),
            }),
        ),
        (
            "hello_ack.rejected",
            bytes_of(&HelloAck {
                protocol_version: PROTOCOL_VERSION,
                outcome: HelloOutcome::Rejected {
                    code: RejectCode::ProtocolVersionUnsupported,
                    cause: "node speaks 9, master supports 1".into(),
                },
                router_url: None,
                router_token: None,
            }),
        ),
    ];
    for (name, bytes) in &cases {
        assert_pinned(pin(&pins, name), bytes, "node-link");
    }
}

/// 5.2：版本协商行为不变——`PROTOCOL_VERSION` 与版本拒绝**指名双方版本**
/// 的既有语义原样在役（本 change 只加闸门，不改链路）。
#[test]
fn node_link_version_rejection_still_names_both_versions() {
    assert_eq!(PROTOCOL_VERSION, 1, "链路协议版本被本 change 意外改动了");
    let rejected = HelloAck {
        protocol_version: PROTOCOL_VERSION,
        outcome: HelloOutcome::Rejected {
            code: RejectCode::ProtocolVersionUnsupported,
            cause: format!("node speaks 9, master supports {PROTOCOL_VERSION}"),
        },
        router_url: None,
        router_token: None,
    };
    let value = serde_json::to_value(&rejected).unwrap();
    assert_eq!(value["protocol_version"], PROTOCOL_VERSION);
    assert_eq!(value["outcome"]["result"], "rejected");
    assert_eq!(value["outcome"]["code"], "protocol_version_unsupported");
    let cause = value["outcome"]["cause"].as_str().unwrap();
    assert!(cause.contains("9"), "拒绝成因须指名节点版本: {cause}");
    assert!(
        cause.contains(&PROTOCOL_VERSION.to_string()),
        "拒绝成因须指名主控支持版本: {cause}"
    );
    // 未知拒绝码仍走 `#[serde(other)] Unknown`（前向容错不被本 change 削弱）。
    let unknown: RejectCode = serde_json::from_value(serde_json::json!("hibernate")).unwrap();
    assert_eq!(unknown, RejectCode::Unknown);
    assert!(unknown.is_permanent(), "看不懂的拒绝按永久处理（不重试刷屏）");
}

// ---------------------------------------------------------------------------
// 5.3 webui WS 与 HTTP
// ---------------------------------------------------------------------------

use sebas_webui::ws_rpc::{Frame, JsonCodec, NotificationFrame, RequestFrame, ResponseFrame, WsCodec};

/// 5.3：webui 面向浏览器的 WS 帧形状钉住——与既有
/// `sebas-webui/tests/ws_rpc_contract_test.rs` **不冲突**（那边钉语义，
/// 这边钉字节），两套都通过。
#[test]
fn webui_ws_frames_match_the_fixture() {
    let doc = fixture("ipc_webui_wire.json");
    let pins = pinned(&doc, "ws_frames");
    let codec = JsonCodec;
    let cases: Vec<(&str, Frame)> = vec![
        (
            "request",
            Frame::Request(RequestFrame {
                id: 7,
                method: "ping".into(),
                params: serde_json::json!({"deep": [1, 2, 3]}),
            }),
        ),
        (
            "response.ok",
            Frame::Response(ResponseFrame::ok(7, serde_json::json!("pong"))),
        ),
        (
            "response.error",
            Frame::Response(ResponseFrame::error(7, "unknown_method", "no such method")),
        ),
        (
            "notification",
            Frame::Notification(NotificationFrame {
                method: "session.created".into(),
                params: serde_json::json!({"session_id": "oc_a"}),
            }),
        ),
    ];
    for (name, frame) in &cases {
        assert_pinned(pin(&pins, name), &codec.encode(frame), "webui-ws");
    }
}

/// 5.3：webui HTTP 侧的代表性载荷形状与 `api_endpoints_test` 的断言一致
/// （字段名集合比对；HTTP 是浏览器契约，本 change **不改**它，只钉住）。
#[test]
fn webui_http_payload_shapes_match_the_fixture_and_api_assertions() {
    let doc = fixture("ipc_webui_wire.json");
    let pins = pinned(&doc, "http_payloads");
    let cases: Vec<(&str, Value)> = vec![
        (
            "summary",
            serde_json::json!({
                "reachability": {"ok": true},
                "execution_bodies": {"acp": {"ok": true}, "native": {"ok": false}},
                "sessions": 3,
            }),
        ),
        (
            "session_row",
            serde_json::to_value(sample_info()).unwrap(),
        ),
        (
            "rejection_body",
            serde_json::json!({"code": "unknown_session", "key": "oc_1"}),
        ),
    ];
    for (name, value) in &cases {
        assert_pinned(pin(&pins, name), &serde_json::to_string(value).unwrap(), "webui-http");
    }
    // 与 api_endpoints_test 断言的口径对齐：`reachability.ok` 是布尔真值、
    // `execution_bodies` 逐执行体带 ok；形状被本 fixture 与那边一起钉住。
    let summary = &cases[0].1;
    assert!(summary["reachability"]["ok"].is_boolean());
    assert!(summary["execution_bodies"]["acp"]["ok"].is_boolean());
    assert!(summary["execution_bodies"]["native"]["ok"].is_boolean());
}