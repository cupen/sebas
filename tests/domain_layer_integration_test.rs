//! add-domain-layer 集成断言（跨 crate 边界，单元测试盖不到的层）。
//!
//! 单元测试已钉住域层内部形状（`sebas-domain` 各模块 + `sebas-channels`
//! key.rs 的黄金样本）；本文件钉住 spec 里只有**跨 crate 视角**才成立的
//! 三条契约：
//!
//! 1. 「session key encoding has exactly one implementation」/「adding a
//!    consumer adds no copy」：各角色 crate 的既有公开路径与唯一实现产出
//!    **逐字节相同**；任务 2.1 审计表里登记过的回退分歧（dispatch 保留
//!    feishu 回退、canonical 严格解码）也在此钉住——分歧是局部策略，不是
//!    第二份编解码。
//! 2. 「a shared protocol crate can be built on the layer without a cycle」：
//!    core 通道 NDJSON 帧的全部词表**仅凭 `sebas_domain` 路径**即可构造并
//!    serde 往返——协议 crate 若按 unify-ipc-protocol-home 抽出去，只依赖
//!    域层即可、不依赖任何角色实现，环无从形成。帧级序列化形状同时长期
//!    承保「golden wire fixtures are unchanged」（任务 5.1 的一次性黄金
//!    diff 之外的可回归载体）。
//! 3. 「shape boundaries are typed and pinned」：对话条目呈现视图与
//!    canonical `TurnEntry` 在**序列化层**逐字节对齐（单测只钉了逐字段
//!    等价；序列化形状等价是更强断言——单侧加字段即失败）。

use sebas_channels::key::{decode_session_key, encode_session_key};
use sebas_channels::ChannelKey;
use sebas_domain::node::NodeView;
use sebas_domain::session::{
    PermissionDecision, PermissionNotice, PendingApproval, PendingDisposition, PendingReason,
    PendingSubmission, RemoteSessionView, SessionEvent, SessionIdentity, SessionInfo,
    SessionRejection, TurnEntry, TurnStreamEvent,
};

/// 审计表（任务 2.1）覆盖的五类 wire 键：普通 web、飞书复合 reference、
/// node_link 嵌套行键、percent 边界、unicode。
fn sample_keys() -> Vec<(&'static str, &'static str)> {
    vec![
        ("web", "web-1"),
        ("feishu", "oc_x\0t1"),
        ("node", "nodeA\0sess-1"),
        ("web", "a b/c?d=e&f+g%"),
        ("web", "中文引用"),
    ]
}

fn domain_session_info(channel: &str, reference: &str) -> SessionInfo {
    SessionInfo {
        channel: channel.into(),
        key: reference.into(),
        session_id: Some("sess-1".into()),
        status: "active".into(),
        phase: Some("OnIt".into()),
        user_prompt: Some("hi".into()),
        last_active_unix: 1_700_000_000,
        project_dir: None,
        current_model: None,
        available_models: None,
        agent_kind: None,
        usage: None,
        backend: Some("acp".into()),
        pending: vec![],
        remote: None,
        desired_mode: "edit".into(),
        effective_mode: None,
        msg_count: 1,
        available_commands: vec![],
        turn_engaged: false,
        spawn_failure_reason: None,
        parked_approvals: 0,
        label: None,
    }
}

// ---- 1. 会话键编解码：一份实现，多角色同字节 ------------------------------

/// spec「exactly one implementation」：dispatch engine 的既有公开路径、
/// webui 呈现行的 encoded_key 与 canonical 实现对同一批键产出**逐字节相同**
/// 的编码，且两条解码路径都能还原（S1.1/S1.2 的跨 crate 半边；黄金字面量
/// 钉住与收敛前实现的逐字节一致）。
#[test]
fn codec_outputs_are_byte_identical_across_consumer_paths() {
    // 黄金字面量（收敛前 dispatch 手写 encoder 的输出形态）。
    assert_eq!(
        encode_session_key(&ChannelKey::feishu("oc_x", Some("t1"))),
        "feishu%00oc_x%00t1"
    );
    assert_eq!(
        encode_session_key(&ChannelKey::new("node", "nodeA\0sess-1")),
        "node%00nodeA%00sess-1"
    );

    for (channel, reference) in sample_keys() {
        let key = ChannelKey::new(channel, reference);
        let canonical = encode_session_key(&key);

        // dispatch engine 的既有公开路径（原 encode_key，native_dispatch_bridge
        // 等调用方仍从这里走）＝唯一实现的原样再导出。
        assert_eq!(
            sebas_dispatch::engine::encode_key(&key),
            canonical,
            "dispatch engine 偏离唯一实现: {channel}/{reference}"
        );

        // webui 呈现行（From<&SessionInfo> 内部走 encode_channel_key）。
        let row = sebas_webui::models::SessionRow::from(&domain_session_info(
            channel, reference,
        ));
        assert_eq!(
            row.encoded_key, canonical,
            "webui 会话行偏离唯一实现: {channel}/{reference}"
        );

        // 两条解码路径都还原原键。
        assert_eq!(decode_session_key(&canonical).unwrap(), key);
        assert_eq!(sebas_dispatch::engine::decode_key(&canonical).unwrap(), key);
    }
}

/// 任务 2.1 审计表 ①/④ 的登记分歧：wire 上的键（出自唯一 encoder，必含
/// NUL）两条解码路径一致；非 wire 输入（无 NUL）canonical 严格拒绝、
/// dispatch 保留旧 feishu 回退、im 侧保留原文回退。钉住它 = 回退策略的
/// 去留是显式决定，不是收敛时顺手改掉的。
#[test]
fn documented_decode_fallback_divergence_is_pinned() {
    // 无 NUL 的非 wire 输入：canonical 严格拒绝。
    assert_eq!(decode_session_key("oc_plain"), None);
    // dispatch 保留旧 feishu 回退：整段（percent 解码后）当飞书 reference。
    assert_eq!(
        sebas_dispatch::engine::decode_key("oc_plain"),
        Some(ChannelKey::feishu("oc_plain", None))
    );
    assert_eq!(
        sebas_dispatch::engine::decode_key("oc%20spaced"),
        Some(ChannelKey::feishu("oc spaced", None))
    );
    // 非法转义：回退路径同样拒绝（percent_decode 严格语义）。
    assert_eq!(sebas_dispatch::engine::decode_key("%ZZ"), None);
    // wire 形态上两路永不分歧。
    let key = ChannelKey::feishu("oc_x", Some("t1"));
    let enc = encode_session_key(&key);
    assert_eq!(sebas_dispatch::engine::decode_key(&enc), Some(key.clone()));
    assert_eq!(decode_session_key(&enc), Some(key));
}

// ---- 2. 协议词表凭域层即可成立（无环前提）--------------------------------

/// spec「a shared protocol crate can be built on the layer without a cycle」：
/// core 通道会出现的每一种会话域词表类型都只经 `sebas_domain` 路径构造并
/// serde 往返。若协议 crate 从根 crate 抽出，它只依赖 sebas-domain——不碰
/// dispatch / webui / router / im，环无从形成。
#[test]
fn core_channel_vocabulary_round_trips_through_domain_paths_alone() {
    let info = domain_session_info("web", "web-1");

    // SessionEvent 全变体（type tag + 载荷往返）。
    let events = vec![
        SessionEvent::Created { session: info.clone() },
        SessionEvent::Updated { session: info.clone() },
        SessionEvent::Removed { channel: "web".into(), key: "web-1".into() },
        SessionEvent::PendingDropped {
            channel: "web".into(),
            key: "web-1".into(),
            dropped: vec![PendingSubmission {
                id: 1,
                text: "t".into(),
                position: 0,
                disposition: PendingDisposition::Turn,
                priority: false,
            }],
        },
        SessionEvent::TurnStalled { channel: "web".into(), key: "web-1".into(), released: 2 },
        SessionEvent::Resync,
    ];
    for e in &events {
        let v = serde_json::to_value(e).unwrap();
        assert_eq!(v["type"], serde_json::from_value::<SessionEvent>(v.clone()).unwrap()
            .tag_name());
        let back: SessionEvent = serde_json::from_value(v).unwrap();
        assert_eq!(&back, e);
    }

    // TurnEntry / TurnStreamEvent / SessionIdentity / RemoteSessionView。
    let entry = TurnEntry::tool(3, "📖 x").with_title("Read · a");
    let stream = TurnStreamEvent {
        channel: "web".into(),
        key: "web-1".into(),
        entries: vec![entry],
    };
    let back: TurnStreamEvent =
        serde_json::from_value(serde_json::to_value(&stream).unwrap()).unwrap();
    assert_eq!(back, stream);

    let identity = SessionIdentity {
        agent_kind: Some("claude".into()),
        desired_mode: Some("ask".into()),
        current_model: None,
        available_models: None,
    };
    let back: SessionIdentity =
        serde_json::from_value(serde_json::to_value(&identity).unwrap()).unwrap();
    assert_eq!(back, identity);

    let remote = RemoteSessionView {
        node_id: "node-a".into(),
        node_status: "online".into(),
        node_cause: None,
        desired_mode: Some("ask".into()),
        effective_mode: Some("ask".into()),
        parked_approvals: 1,
        desired_provider: None,
        provider: None,
        provider_cause: None,
    };
    let back: RemoteSessionView =
        serde_json::from_value(serde_json::to_value(&remote).unwrap()).unwrap();
    assert_eq!(back, remote);

    // 审批词表（PendingApproval / PermissionNotice / PermissionDecision）。
    let approval = PendingApproval {
        request_id: "claude:tc-1".into(),
        tool_name: "Bash".into(),
        args: serde_json::json!({"cmd": "ls"}),
    };
    let notice = PermissionNotice {
        request_id: approval.request_id.clone(),
        session_id: encode_session_key(&ChannelKey::new("web", "web-1")),
        tool_name: approval.tool_name.clone(),
        args: approval.args.clone(),
        reason: "gated".into(),
    };
    let back: PermissionNotice =
        serde_json::from_value(serde_json::to_value(&notice).unwrap()).unwrap();
    assert_eq!(back, notice);

    for d in [
        PermissionDecision::AllowOnce,
        PermissionDecision::AllowSession,
        PermissionDecision::Deny,
        PermissionDecision::Escalate { reason: "r".into() },
    ] {
        let back: PermissionDecision =
            serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }

    // 类型化拒绝全变体（code tag 词表）。
    let rejections = vec![
        (SessionRejection::UnknownSession { key: "k".into() }, "unknown_session"),
        (SessionRejection::UnusableProjectDir, "unusable_project_dir"),
        (SessionRejection::Capacity { limit: 7 }, "capacity"),
        (SessionRejection::Unavailable { cause: "c".into() }, "unavailable"),
        (
            SessionRejection::BackendUnavailable { backend: "b".into(), cause: "c".into() },
            "backend_unavailable",
        ),
        (SessionRejection::QueueFull { limit: 3 }, "queue_full"),
        (
            SessionRejection::PendingRejected { reason: PendingReason::AlreadyStarted },
            "pending_rejected",
        ),
        (SessionRejection::Idle { key: "k".into() }, "idle_session"),
    ];
    for (r, tag) in &rejections {
        let v = serde_json::to_value(r).unwrap();
        assert_eq!(&v["code"], tag);
        let back: SessionRejection = serde_json::from_value(v).unwrap();
        assert_eq!(&back, r);
    }
}

/// SessionEvent 的 tag 读取辅助（上面的断言用）。
trait TagName {
    fn tag_name(&self) -> &'static str;
}
impl TagName for SessionEvent {
    fn tag_name(&self) -> &'static str {
        match self {
            SessionEvent::Created { .. } => "created",
            SessionEvent::Updated { .. } => "updated",
            SessionEvent::Removed { .. } => "removed",
            SessionEvent::PendingDropped { .. } => "pending_dropped",
            SessionEvent::TurnStalled { .. } => "turn_stalled",
            SessionEvent::Resync => "resync",
        }
    }
}

// ---- 3. 帧级 wire 形状（R3 的可回归载体）---------------------------------

/// spec「golden wire fixtures are unchanged」在 core 通道 NDJSON 帧级的
/// 长期钉：Snapshot / Rejected / NodeLink(Nodes) 三种帧与重构前逐字段一致
/// ——尤其 NodeView 合一后远端行**不带 local 键**（任务 4.1 的承诺），
/// desired_mode 恒上 wire、可选键缺席。
#[test]
fn core_channel_frames_keep_their_pre_refactor_wire_shape() {
    use sebas::core_channel::protocol::{CoreChannelResponse, NodeLinkOutcome};

    // Snapshot 帧：SessionInfo 形状（域层形状钉的帧级镜像）。
    let resp = CoreChannelResponse::Snapshot {
        sessions: vec![domain_session_info("web", "web-1")],
    };
    let v = serde_json::to_value(&resp).unwrap();
    assert_eq!(v["cmd"], "snapshot");
    let s = &v["sessions"][0];
    assert_eq!(s["desired_mode"], "edit", "desired_mode 恒上 wire（D5b）");
    assert_eq!(s["backend"], "acp");
    assert!(s.get("remote").is_none(), "None 遥视图不上 wire");
    assert!(s.get("label").is_none());
    assert!(s.get("spawn_failure_reason").is_none());
    let back: CoreChannelResponse = serde_json::from_value(v).unwrap();
    assert_eq!(back, resp);

    // Rejected 帧：typed rejection flatten，code/载荷不换壳。
    let resp = CoreChannelResponse::Rejected {
        rejection: SessionRejection::Capacity { limit: 7 },
    };
    let v = serde_json::to_value(&resp).unwrap();
    assert_eq!(v["cmd"], "rejected");
    assert_eq!(v["code"], "capacity");
    assert_eq!(v["limit"], 7);
    let back: CoreChannelResponse = serde_json::from_value(v).unwrap();
    assert_eq!(back, resp);

    // NodeLink(Nodes) 帧：远端 NodeView 行不带 local 键（合并前 core 通道
    // 上的 JSON 逐字节一致——add-domain-layer 4.1）。
    let resp = CoreChannelResponse::NodeLink(NodeLinkOutcome::Nodes {
        nodes: vec![NodeView {
            id: "node-a".into(),
            status: "online".into(),
            last_seen_unix: Some(1_700_000_000),
            created_unix: 1_600_000_000,
            local: false,
        }],
    });
    let v = serde_json::to_value(&resp).unwrap();
    assert_eq!(v["cmd"], "node_link");
    assert_eq!(v["result"], "nodes");
    let node = &v["nodes"][0];
    assert_eq!(node["id"], "node-a");
    assert_eq!(node["last_seen_unix"], 1_700_000_000);
    assert!(
        node.get("local").is_none(),
        "远端节点的 local:false 不上 wire（合并前形状）：{node}"
    );
    let back: CoreChannelResponse = serde_json::from_value(v).unwrap();
    assert_eq!(back, resp);
}

// ---- 4. 呈现视图 ↔ canonical 的序列化层对齐（S4.1 的强断言）--------------

/// spec「a test pins both serialized shapes, so that a field added on one
/// side alone fails the test」：`ConversationEntryView` 与 canonical
/// `TurnEntry` 不仅逐字段等价，**序列化结果也相等**——任何一侧单独加
/// 字段（可选键或恒上 wire 键）都让本测试失败。
#[test]
fn conversation_entry_view_serializes_identically_to_canonical_turn_entry() {
    // 带可选键的完整条目（tool + title + failure_class）。
    let canonical = TurnEntry {
        position: 2,
        kind: "content".into(),
        element_type: "tool".into(),
        content: "📖 x".into(),
        created_at_unix: 42,
        title: Some("Read · a".into()),
        failure_class: Some("generic".into()),
    };
    let view = sebas_webui::models::ConversationEntryView::from(&canonical);
    assert_eq!(
        serde_json::to_value(&view).unwrap(),
        serde_json::to_value(&canonical).unwrap(),
        "呈现视图与 canonical 的序列化形状必须逐字节对齐"
    );

    // 最小条目：两侧的可选键（title / failure_class）都缺席。
    let minimal = TurnEntry::markdown(0, "hi");
    let view = sebas_webui::models::ConversationEntryView::from(&minimal);
    let v = serde_json::to_value(&view).unwrap();
    assert!(v.get("title").is_none() && v.get("failure_class").is_none());
    assert_eq!(v, serde_json::to_value(&minimal).unwrap());
}
