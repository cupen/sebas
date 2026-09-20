//! fix-webui-approval-restore-and-session-identity 的引擎级单测。
//!
//! 覆盖（纯引擎路径，不拉子进程）：
//! - 1.1 泊车审批读模型：泊车后枚举可见（request_id/tool/args）、批复后消失；
//! - 1.4/2.3 fail-closed：未泊车 / 已释放的 request_id 查不到归属（批复路由
//!   据此拒绝）；
//! - 2.1 cancel 释放泊车：审批挂起时可取消、pending 清空、`turn_engaged`
//!   回落、重复 cancel 无害；
//! - 2.2 停止条目：被打标回合的 `Finished` append「回合被停止」错误类条目，
//!   正常完成回合不含；
//! - 3.2 归档恢复身份：restore 带身份四项落映射、旧档（全空）维持现默认；
//! - 5.1 会话 label：设置/清空随快照可见、未知会话拒绝。

/// 相位快照直读（测试可见性；经公开的 card_state_snapshot）。
async fn card_phase(router: &DispatchHandle, session_id: &str) -> Option<String> {
    router
        .card_state_snapshot()
        .await
        .remove(session_id)
        .map(|st| st.status_emoji)
}

use sebas_acp::claude::session::{AcpCommand, AcpEvent, Decision};
use sebas_channels::ChannelKey;
use sebas_dispatch::card_state::phase;
use sebas_dispatch::engine::{DispatchHandle, Out};
use sebas_dispatch::state::{Mapping, SessionIdentity, SessionMap};
use sebas_dispatch::{SessionInfo, TurnEntry};

fn web_key(tag: &str) -> ChannelKey {
    ChannelKey::new("web", format!("appr-{tag}"))
}

/// 造一个活跃会话并泊车一条权限请求（dispatch_acp_event 直达路径）。
async fn parked_session(router: &DispatchHandle, tag: &str, sid: &str, req_id: &str) -> ChannelKey {
    let key = web_key(tag);
    router
        .map
        .insert(key.clone(), Mapping::active(sid))
        .await
        .unwrap();
    router
        .dispatch_acp_event(AcpEvent::PermissionRequest {
            session_id: sid.to_string(),
            request_id: req_id.to_string(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"command": "rm -rf build"}),
        })
        .await;
    key
}

/// 1.1 读模型主契约：泊车后枚举可见（含 request_id/tool/args），批复出站
/// 后消失。
#[tokio::test]
async fn parked_requests_are_listed_and_clear_after_the_reply() {
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    let key = parked_session(&router, "read", "s-read", "claude:tc-1").await;

    // 泊车中：读模型可见、字段齐全。
    let listed = router
        .pending_permission_requests(&key)
        .await
        .expect("known session");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].request_id, "claude:tc-1");
    assert_eq!(listed[0].tool_name, "Bash");
    assert_eq!(listed[0].args["command"], "rm -rf build");

    // 批复出站（emit 的 PermissionReply 钩子解除泊车）。
    router
        .emit(Out::SendAcp {
            session_id: "s-read".into(),
            cmd: AcpCommand::PermissionReply {
                session_id: "s-read".into(),
                request_id: "claude:tc-1".into(),
                decision: Decision::AllowOnce,
            },
        })
        .await;
    let listed = router
        .pending_permission_requests(&key)
        .await
        .expect("known session");
    assert!(listed.is_empty(), "decided request must leave the read model");

    // Out 通道里确实有一条 PermissionReply（批复真的走了）；泊车时先发的
    // SendCard 仍排在队头，向后扫到 reply 为止。
    let mut saw_reply = false;
    for _ in 0..4 {
        match out_rx.recv().await {
            Some(Out::SendAcp {
                cmd: AcpCommand::PermissionReply { request_id, .. },
                ..
            }) => {
                assert_eq!(request_id, "claude:tc-1");
                saw_reply = true;
                break;
            }
            Some(_) => continue,
            None => break,
        }
    }
    assert!(saw_reply, "the reply must leave through the out channel");
}

/// 1.1 补口：未知会话读模型返回 None（路由转 404）；泊车在等的会话
/// `turn_engaged = true`（waiting 投影事实）。
#[tokio::test]
async fn read_model_none_for_unknown_session_and_engaged_while_parked() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let key = parked_session(&router, "engage", "s-engage", "req-e1").await;

    assert!(
        router
            .pending_permission_requests(&web_key("unknown"))
            .await
            .is_none(),
        "unknown session yields None (route maps to 404)"
    );

    let info: SessionInfo = router.session_info_for(&key).await.expect("info");
    assert!(info.turn_engaged, "parked approval counts as engaged");
    assert_eq!(info.parked_approvals, 1);
}

/// 2.1 主契约：审批挂起时 cancel——在飞判定命中（泊车也算占用）、pending
/// 清空、`turn_engaged` 回落 false、重复 cancel 无害（Idle）。
#[tokio::test]
async fn cancel_releases_parked_approvals_and_resets_engagement() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let key = parked_session(&router, "cancel", "s-cancel", "req-c1").await;

    // 泊车在等：泊车可见。
    assert_eq!(
        router
            .pending_permission_requests(&key)
            .await
            .unwrap()
            .len(),
        1
    );

    // 第一次 cancel：泊车中的回合同样可停（Dispatched），泊车被释放。
    let outcome = router.web_cancel_session(&key).await;
    assert!(
        matches!(outcome, sebas_dispatch::engine::CancelOutcome::Dispatched),
        "a parked turn is cancellable"
    );
    assert!(
        router
            .pending_permission_requests(&key)
            .await
            .unwrap()
            .is_empty(),
        "cancel releases every parked request (fail-closed)"
    );
    let info = router.session_info_for(&key).await.expect("info");
    assert!(
        !info.turn_engaged,
        "turn_engaged falls back to false once nothing is parked and no phase is in flight"
    );

    // 重复 cancel：无事可停（Idle）——幂等，不再伪造成功也不炸。
    let outcome = router.web_cancel_session(&key).await;
    assert!(matches!(outcome, sebas_dispatch::engine::CancelOutcome::Idle));
}

/// 2.3 fail-closed：已释放 request_id 的归属查不到（`permission_parked_session`
/// 为批复路由的拒绝依据），解除动作本身不再可达。
#[tokio::test]
async fn released_request_id_is_no_longer_answerable() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let key = parked_session(&router, "release", "s-release", "req-r1").await;

    assert_eq!(
        router.permission_parked_session("req-r1").await.as_deref(),
        Some("s-release"),
        "while parked, the request is answerable"
    );

    router.web_cancel_session(&key).await;
    assert_eq!(
        router.permission_parked_session("req-r1").await,
        None,
        "released id must not be answerable afterwards"
    );
}

/// （fix-webui-qa-defects-round4 2.3）接收回执相位的提交走既有 turn-queue：
/// SEED+prompt 的在飞窗口内的新提交入队（不重置卡态、不二次 SendAcp 顶掉
/// 在飞回合），与 WORKING 相位同一 back-pressure 语义。空 prompt 的 SEED
/// （resume 激活语义）不算在飞——照常开轮。
#[tokio::test]
async fn submission_during_receipt_phase_queues_instead_of_interleaving() {
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    let key = web_key("receipt-q");
    router
        .map
        .insert(key.clone(), Mapping::active("s-rcpt"))
        .await
        .unwrap();
    // 接收回执相位：卡态 SEED + prompt（提交已接受、首帧未落）。
    router.seed_card("s-rcpt".to_string(), "first prompt".into()).await;

    // 在飞判定：turn_engaged 为 true（spec「accepted-receipt phase」）。
    let info = router.session_info_for(&key).await.expect("info");
    assert!(
        info.turn_engaged,
        "the receipt phase counts as turn-occupied"
    );

    // 新提交：入队而非开新轮。
    router
        .web_send_message(key.clone(), "second during receipt".into())
        .await
        .expect("accepted");
    assert_eq!(
        router.map.queue_len(&key).await,
        1,
        "the submission must be enqueued during the receipt phase"
    );
    // 卡态未被重置（prompt 仍是首条提交），transcript 只有一轮的 prompt。
    assert_eq!(
        card_phase(&router, "s-rcpt").await.as_deref(),
        Some(phase::SEED)
    );
    let turns = router.session_turns(&key, 0).await.unwrap();
    assert_eq!(
        turns.iter().filter(|e| e.kind == "prompt").count(),
        1,
        "no second prompt entry may land while the first turn is a receipt: {turns:?}"
    );
    // 排空 Out 通道，确认没有第二次 SendAcp/开轮卡。
    let mut saw_continue = false;
    while let Ok(Some(o)) =
        tokio::time::timeout(std::time::Duration::from_millis(60), out_rx.recv()).await
    {
        if matches!(
            o,
            Out::SendAcp {
                cmd: AcpCommand::ContinueSession { .. },
                ..
            }
        ) {
            saw_continue = true;
        }
    }
    assert!(
        !saw_continue,
        "the receipt-phase submission must not drive a second turn"
    );

    // 空 prompt 的 SEED（激活语义）不在飞：新提交照常开轮（SendAcp 发出）。
    let key_idle = web_key("receipt-idle");
    router
        .map
        .insert(key_idle.clone(), Mapping::active("s-rcpt-idle"))
        .await
        .unwrap();
    router.seed_card("s-rcpt-idle".to_string(), String::new()).await;
    router
        .web_send_message(key_idle.clone(), "first real turn".into())
        .await
        .expect("accepted");
    assert_eq!(
        router.map.queue_len(&key_idle).await,
        0,
        "an activated-idle session must start the turn, not queue"
    );
}

/// 2.2 主契约：被打标回合的 `Finished` append「回合被停止」错误类条目；
/// 正常完成回合不含该条目。
#[tokio::test]
async fn cancelled_turn_appends_a_stop_entry_and_normal_finish_does_not() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());

    // 回合 A：cancel 打标 → Finished = 有停止条目。
    let key_a = web_key("stop-a");
    router
        .map
        .insert(key_a.clone(), Mapping::active("s-a"))
        .await
        .unwrap();
    // 无卡态（真空闲）→ Idle、不打标（无事可停不算取消）。
    // （round4 2.3：带 prompt 的 SEED 卡 = 接收回执相位，可停——见下。）
    let outcome = router.web_cancel_session(&key_a).await;
    assert!(matches!(outcome, sebas_dispatch::engine::CancelOutcome::Idle));

    // 接收回执相位（SEED + prompt 已随开轮记入卡态）同样可停（round4 2.3，
    // spec「accepted receipt without agent output offers stop」的引擎半边）。
    router.seed_card("s-a".to_string(), "run".into()).await;
    let outcome = router.web_cancel_session(&key_a).await;
    assert!(matches!(
        outcome,
        sebas_dispatch::engine::CancelOutcome::Dispatched
    ));

    // 让回合 A 真正进入在飞（SEED→WORKING）再取消。
    router
        .dispatch_acp_event(AcpEvent::TextDelta {
            session_id: "s-a".into(),
            delta: "streaming".into(),
        })
        .await;
    assert_eq!(
        card_phase(&router, "s-a").await.as_deref(),
        Some(phase::WORKING)
    );
    let outcome = router.web_cancel_session(&key_a).await;
    assert!(matches!(
        outcome,
        sebas_dispatch::engine::CancelOutcome::Dispatched
    ));
    router
        .apply_event(
            "s-a",
            &AcpEvent::Finished {
                session_id: "s-a".into(),
            },
        )
        .await;
    let turns = router.session_turns(&key_a, 0).await.unwrap();
    let stop_entries: Vec<&TurnEntry> = turns
        .iter()
        .filter(|e| e.element_type == "error" && e.content.contains("回合被停止"))
        .collect();
    assert_eq!(
        stop_entries.len(),
        1,
        "exactly one stop entry after a cancelled turn"
    );
    assert_eq!(
        stop_entries[0].failure_class.as_deref(),
        Some(sebas_dispatch::failure_class::GENERIC),
        "the stop entry is an error-class entry"
    );

    // 回合 B：正常完成（无 cancel 打标）→ 无停止条目。
    let key_b = web_key("stop-b");
    router
        .map
        .insert(key_b.clone(), Mapping::active("s-b"))
        .await
        .unwrap();
    router.seed_card("s-b".to_string(), "run".into()).await;
    router
        .dispatch_acp_event(AcpEvent::TextDelta {
            session_id: "s-b".into(),
            delta: "streaming".into(),
        })
        .await;
    router
        .apply_event(
            "s-b",
            &AcpEvent::Finished {
                session_id: "s-b".into(),
            },
        )
        .await;
    let turns = router.session_turns(&key_b, 0).await.unwrap();
    assert!(
        !turns
            .iter()
            .any(|e| e.content.contains("回合被停止")),
        "a normally finished turn must not carry a stop entry"
    );
}

/// 3.2 主契约：restore 携带身份四项——恢复后 `agent_kind` / desired mode /
/// model 面与归档前一致；旧档（全空身份）维持现默认。
#[tokio::test]
async fn restore_rebuilds_the_session_identity_and_legacy_entries_fall_back() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());

    // 带身份恢复。
    let key = web_key("identity");
    let identity = SessionIdentity {
        agent_kind: Some("claude".into()),
        desired_mode: Some("auto".into()),
        current_model: Some("claude-sonnet-4".into()),
        available_models: Some(vec!["claude-sonnet-4".into(), "claude-opus-4".into()]),
    };
    router
        .web_restore_session(
            key.clone(),
            Some("old-sid".into()),
            Some("/proj".into()),
            Vec::new(),
            identity,
            None,
            None,
        )
        .await
        .expect("restore must succeed");
    let info = router.session_info_for(&key).await.expect("info");
    assert_eq!(info.agent_kind.as_deref(), Some("claude"));
    assert_eq!(info.desired_mode, "auto");
    assert_eq!(info.current_model.as_deref(), Some("claude-sonnet-4"));
    assert_eq!(
        info.available_models.as_deref(),
        Some(&[
            "claude-sonnet-4".to_string(),
            "claude-opus-4".to_string()
        ][..])
    );

    // 旧档（全空身份）：agent/mode/model 维持既有默认，恢复不炸。
    let legacy = web_key("legacy");
    router
        .web_restore_session(
            legacy.clone(),
            Some("old-2".into()),
            Some("/proj".into()),
            Vec::new(),
            SessionIdentity::default(),
            None,
            None,
        )
        .await
        .expect("legacy restore must succeed");
    let info = router.session_info_for(&legacy).await.expect("info");
    assert_eq!(info.agent_kind, None, "legacy entry keeps the default-agent fallback");
    assert_eq!(
        info.desired_mode,
        sebas_dispatch::engine::ask_mode(),
        "legacy entry falls back to the default mode"
    );
    assert_eq!(info.current_model, None);
    assert_eq!(info.available_models, None);
}

/// 5.1 主契约：label 设置/清空随快照可见；未知会话类型化拒绝；零轮占位
/// 同样可命名。
///
/// （fix-webui-qa-defects-round5 3.1）label 写入成功路径必须广播既有
/// session.updated 事件（订阅端 = WebUI ws 转发层的唯一事件源，五键相位
/// 帧由它驱动；帧形状不变，label 经订阅端的轻量重取刷新行名）。
#[tokio::test]
async fn session_label_sets_clears_and_rejects_unknown_keys() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let mut events = router.subscribe_session_events();

    // 零轮占位（Spawning + awaiting_first_prompt）可命名。
    let placeholder = web_key("placeholder");
    router
        .map
        .insert(placeholder.clone(), Mapping::spawning_with(None, None, None, true))
        .await
        .unwrap();
    router
        .web_set_session_label(placeholder.clone(), Some("重构计划".into()))
        .await
        .expect("placeholder is nameable");

    // 订阅端收到该会话的 Updated，且载荷携带新 label（rail 重取的行真源）。
    let ev = events.recv().await.expect("label write must publish");
    match ev {
        sebas_dispatch::SessionEvent::Updated { session } => {
            assert_eq!(session.channel_key(), placeholder);
            assert_eq!(session.label.as_deref(), Some("重构计划"));
        }
        other => panic!("expected Updated, got {other:?}"),
    }

    let info = router.session_info_for(&placeholder).await.expect("info");
    assert_eq!(info.label.as_deref(), Some("重构计划"));

    // 清空（None）回退：同样发布 Updated（行名回到 prompt 预览的事实源）。
    router
        .web_set_session_label(placeholder.clone(), None)
        .await
        .expect("clearing works");
    let ev = events.recv().await.expect("clear must publish too");
    match ev {
        sebas_dispatch::SessionEvent::Updated { session } => {
            assert_eq!(session.label, None);
        }
        other => panic!("expected Updated, got {other:?}"),
    }
    let info = router.session_info_for(&placeholder).await.expect("info");
    assert_eq!(info.label, None);

    // 未知会话拒绝，且不发布任何事件（拒绝不产生相位帧）。
    assert!(
        router
            .web_set_session_label(web_key("unknown"), Some("x".into()))
            .await
            .is_err(),
        "unknown session must be rejected"
    );
    assert!(
        events.try_recv().is_err(),
        "a rejected label write must not publish"
    );
}
