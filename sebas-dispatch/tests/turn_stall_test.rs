//! 回合停滞看门狗的引擎级单测（fix-pending-queue-liveness 2.1/2.2）。
//!
//! 覆盖 core-session-channel delta 的三类契约场景：
//! - 停滞回合被强制收尾：SEED/WORKING → DONE + drain + TurnStalled 通知
//!   （点名会话与释放的搁浅条目数）；
//! - 泊车审批豁免：泊车中的回合不计时，扫描不收尾；
//! - `timeout = 0` 关闭：扫描短路，行为与无看门狗一致；
//! - 流式中的会话（事件持续到达）永不触发。
//!
//! 纯引擎路径（不拉子进程）：映射直插 + seed_card / apply_event 驱动相位，
//! 时钟经 `rewind_last_event_for_test` 回拨模拟长停滞。

use sebas_acp::claude::session::AcpEvent;
use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_dispatch::card_state::phase;
use sebas_dispatch::engine::{DispatchHandle, Out, SessionEvent};
use sebas_dispatch::state::{Mapping, QueuedTurn, SessionMap};

fn web_key(tag: &str) -> ChannelKey {
    ChannelKey::new("web", format!("stall-{tag}"))
}

/// 造一个 WORKING 相位的活跃会话（seed → TextDelta，事件时钟停在流式帧）。
async fn seed_working_session(router: &DispatchHandle, key: &ChannelKey, sid: &str) {
    router
        .map
        .insert(key.clone(), Mapping::active(sid))
        .await
        .unwrap();
    router.seed_card(sid.to_string(), "run".into()).await;
    router
        .dispatch_acp_event(AcpEvent::TextDelta {
            session_id: sid.to_string(),
            delta: "streaming...".into(),
        })
        .await;
    assert_eq!(
        router.card_state_status_emoji(sid).await.as_deref(),
        Some(phase::WORKING),
        "setup: session must be in the WORKING phase"
    );
}

/// 2.2 主契约：停滞 WORKING 回合被强制收尾到 DONE、drain 开轮下一提交、
/// TurnStalled 事件点名会话与释放条目数。
#[tokio::test]
async fn stalled_turn_force_settles_drains_and_notifies() {
    let map = SessionMap::new();
    let key = web_key("settle");
    let (router, mut out_rx) = DispatchHandle::new(map);
    router.set_turn_stall_timeout(600);
    let mut events = router.subscribe_session_events();

    seed_working_session(&router, &key, "s-stall").await;
    // 两条待执行提交被卡在 WORKING 回合后面。
    router
        .map
        .enqueue_turn(&key, QueuedTurn::new("queued one", None, false))
        .await;
    router
        .map
        .enqueue_turn(&key, QueuedTurn::new("queued two", None, false))
        .await;

    // 模拟长时间无事件（阈值 600s，回拨 1 小时）。
    router
        .stall_registry()
        .rewind_last_event_for_test("s-stall", 3600)
        .await;

    let settled = router.force_settle_stalled_turns().await;
    assert_eq!(
        settled,
        vec![(key.clone(), 2)],
        "the stalled session settles and names the two released submissions"
    );

    // 相位：先收到 DONE（drain 的准入终态），drain 随即开轮队头——卡片经
    // emit_turn_card 重置回 SEED（新回合的种子态）。终态驻留的断言在
    // 「无队列」用例（parked/zero tests）。
    assert_eq!(
        router.card_state_status_emoji("s-stall").await.as_deref(),
        Some(phase::SEED),
        "after the force-settle the queue head must be streaming (fresh SEED card)"
    );

    // drain 开轮队头：出站收到新回合卡 + ContinueSession("queued one")。
    let mut saw_card = false;
    let mut continue_prompt: Option<String> = None;
    for _ in 0..8 {
        let out = tokio::time::timeout(std::time::Duration::from_millis(200), out_rx.recv())
            .await
            .ok()
            .flatten();
        match out {
            Some(Out::SendCard { .. }) => saw_card = true,
            Some(Out::SendAcp {
                cmd:
                    sebas_acp::claude::session::AcpCommand::ContinueSession { prompt, .. },
                ..
            }) => {
                continue_prompt = Some(prompt);
                break;
            }
            _ => {}
        }
    }
    assert!(saw_card, "drain must emit the next per-turn card");
    assert_eq!(
        continue_prompt.as_deref(),
        Some("queued one"),
        "the queue head must start its turn after the force-settle"
    );

    // 通知事实：TurnStalled 点名会话与释放条目数（发布在 Updated 之外）。
    let mut notified: Option<(String, usize)> = None;
    for _ in 0..16 {
        match events.try_recv() {
            Ok(SessionEvent::TurnStalled { key: k, released, .. }) => {
                notified = Some((k, released));
                break;
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    assert_eq!(
        notified,
        Some((key.reference.clone(), 2)),
        "the warn-notice fact must name the session and the released count"
    );

    // transcript 就地点名收尾原因（error 条目可见）。
    let turns = router.session_turns(&key, 0).await.unwrap();
    assert!(
        turns
            .iter()
            .any(|t| t.element_type == "error" && t.content.contains("回合停滞被强制收尾")),
        "the transcript must explain the forced settle: {turns:?}"
    );

    // 队列只弹出队头，剩余条目等新回合结束后继续 drain。
    assert_eq!(router.map.queue_len(&key).await, 1);
}

/// 2.1/D2 泊车豁免：泊在权限请求上的回合不计时——回拨超过阈值也不收尾；
/// 批复（PermissionReply 出站）解除豁免后重新进入扫描视野。
#[tokio::test]
async fn parked_permission_is_exempt_until_it_resolves() {
    let map = SessionMap::new();
    let key = web_key("parked");
    let (router, _out_rx) = DispatchHandle::new(map);
    router.set_turn_stall_timeout(600);

    seed_working_session(&router, &key, "s-parked").await;
    // 权限请求到达：泊车登记（同时该事件刷新时钟，再手动回拨）。
    router
        .dispatch_acp_event(AcpEvent::PermissionRequest {
            session_id: "s-parked".into(),
            request_id: "req-1".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"command": "ls"}),
        })
        .await;
    router
        .stall_registry()
        .rewind_last_event_for_test("s-parked", 3600)
        .await;

    let settled = router.force_settle_stalled_turns().await;
    assert!(
        settled.is_empty(),
        "a turn parked on a permission request must be exempt from the stall guard"
    );
    assert_eq!(
        router.card_state_status_emoji("s-parked").await.as_deref(),
        Some(phase::WORKING),
        "the parked turn must stay untouched"
    );

    // 批复出站（emit 单点）→ 豁免解除 → 同一时钟下看门狗重新看得见它。
    router
        .emit(Out::SendAcp {
            session_id: "s-parked".into(),
            cmd: sebas_acp::claude::session::AcpCommand::PermissionReply {
                session_id: "s-parked".into(),
                request_id: "req-1".into(),
                decision: sebas_acp::Decision::AllowOnce,
            },
        })
        .await;
    let settled = router.force_settle_stalled_turns().await;
    assert_eq!(
        settled.len(),
        1,
        "after the permission resolves the guard must apply again"
    );
    assert_eq!(
        router.card_state_status_emoji("s-parked").await.as_deref(),
        Some(phase::DONE),
    );
}

/// 2.2 `timeout = 0` 关闭：扫描短路，什么都不收尾。
#[tokio::test]
async fn zero_timeout_disables_the_guard() {
    let map = SessionMap::new();
    let key = web_key("off");
    let (router, _out_rx) = DispatchHandle::new(map);
    router.set_turn_stall_timeout(0);

    seed_working_session(&router, &key, "s-off").await;
    router
        .stall_registry()
        .rewind_last_event_for_test("s-off", 86400)
        .await;

    let settled = router.force_settle_stalled_turns().await;
    assert!(settled.is_empty(), "timeout=0 must disable the guard");
    assert_eq!(
        router.card_state_status_emoji("s-off").await.as_deref(),
        Some(phase::WORKING),
        "the working phase must be untouched with the guard off"
    );
}

/// 流式中的会话（事件持续到达重置时钟）永不触发——总时长与看门狗无关。
#[tokio::test]
async fn streaming_events_keep_resetting_the_clock() {
    let map = SessionMap::new();
    let key = web_key("stream");
    let (router, _out_rx) = DispatchHandle::new(map);
    router.set_turn_stall_timeout(600);

    seed_working_session(&router, &key, "s-stream").await;
    // 「一小时的会话」：时钟不断被新事件重置，每次回拨都会被下一次 touch
    // 清零——等价于持续流式。最后一帧到达后立即扫描。
    router
        .stall_registry()
        .rewind_last_event_for_test("s-stream", 3600)
        .await;
    router
        .dispatch_acp_event(AcpEvent::TextDelta {
            session_id: "s-stream".into(),
            delta: "still going".into(),
        })
        .await;

    let settled = router.force_settle_stalled_turns().await;
    assert!(
        settled.is_empty(),
        "a session streaming events must never trip the guard regardless of turn duration"
    );
}

/// 2.3：turn_engaged 快照事实——WORKING ∨ 泊车 ∨ spawn 窗口；终态/休眠不占用。
#[tokio::test]
async fn turn_engaged_reflects_working_parked_and_spawn_window() {
    let map = SessionMap::new();
    let key = web_key("engaged");
    let (router, _out_rx) = DispatchHandle::new(map);

    // spawn 窗口：0-turn 占位（Spawning）= 占用。
    let spawn_key = web_key("engaged-spawn");
    router.map.begin_spawn(spawn_key.clone()).await.unwrap();
    let info = router.session_info_for(&spawn_key).await.unwrap();
    assert!(
        info.turn_engaged,
        "the spawn window counts as turn-occupied"
    );

    // WORKING 相位 = 占用。
    seed_working_session(&router, &key, "s-engaged").await;
    let info = router.session_info_for(&key).await.unwrap();
    assert!(info.turn_engaged, "WORKING phase counts as engaged");

    // 泊车（无 WORKING 相位不可达——泊车发生在回合内；这里验证泊车事实
    // 单独成立：仅泊车登记、无卡片）。
    let parked_key = web_key("engaged-parked");
    router
        .map
        .insert(parked_key.clone(), Mapping::active("s-parked-2"))
        .await
        .unwrap();
    router
        .dispatch_acp_event(AcpEvent::PermissionRequest {
            session_id: "s-parked-2".into(),
            request_id: "req-2".into(),
            tool_name: "Read".into(),
            args: serde_json::json!({"path": "a"}),
        })
        .await;
    let info = router.session_info_for(&parked_key).await.unwrap();
    assert!(
        info.turn_engaged,
        "a session with a parked permission counts as engaged even without a card phase"
    );

    // 终态不占用。
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-engaged".into(),
        })
        .await;
    let info = router.session_info_for(&key).await.unwrap();
    assert!(
        !info.turn_engaged,
        "a settled session is no longer turn-occupied"
    );
}

/// SEED 相位的停滞同样被收尾（「开轮后一帧内容都没出就沉默」类——如子进程
/// 秒死、refusal 帧丢失）：收尾锚定含 SEED（与 `AcpEvent::Error {
/// terminal: false }` 臂的收尾契约同款），drain 照常开轮队头。
#[tokio::test]
async fn seed_phase_stall_is_also_force_settled_and_drains() {
    let map = SessionMap::new();
    let key = web_key("seed-stall");
    let (router, mut out_rx) = DispatchHandle::new(map);
    router.set_turn_stall_timeout(600);
    let mut events = router.subscribe_session_events();

    router
        .map
        .insert(key.clone(), Mapping::active("s-seed-stall"))
        .await
        .unwrap();
    // 开轮（SEED）：只种卡，无任何内容事件到达。
    router
        .seed_card("s-seed-stall".to_string(), "silent start".into())
        .await;
    assert_eq!(
        router
            .card_state_snapshot()
            .await
            .remove("s-seed-stall")
            .map(|st| st.status_emoji)
            .as_deref(),
        Some(phase::SEED),
        "setup: the card must sit in the SEED phase"
    );
    router
        .map
        .enqueue_turn(&key, QueuedTurn::new("queued behind", None, false))
        .await;

    router
        .stall_registry()
        .rewind_last_event_for_test("s-seed-stall", 3600)
        .await;
    let settled = router.force_settle_stalled_turns().await;
    assert_eq!(
        settled,
        vec![(key.clone(), 1)],
        "a SEED-phase stall settles and releases the queued submission"
    );

    // drain 开轮队头：出站收到 ContinueSession("queued behind")。
    let mut continue_prompt: Option<String> = None;
    for _ in 0..8 {
        let out = tokio::time::timeout(std::time::Duration::from_millis(200), out_rx.recv())
            .await
            .ok()
            .flatten();
        if let Some(Out::SendAcp {
            cmd: sebas_acp::claude::session::AcpCommand::ContinueSession { prompt, .. },
            ..
        }) = out
        {
            continue_prompt = Some(prompt);
            break;
        }
    }
    assert_eq!(
        continue_prompt.as_deref(),
        Some("queued behind"),
        "the queue head must start its turn after the SEED-phase force-settle"
    );

    // TurnStalled 通知照发（点名会话与释放条目数）。
    let mut notified: Option<usize> = None;
    for _ in 0..16 {
        match events.try_recv() {
            Ok(SessionEvent::TurnStalled { key: k, released, .. }) if k == key.reference => {
                notified = Some(released);
                break;
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    assert_eq!(notified, Some(1), "the notice must fire for a SEED stall too");
}

/// 状态迁移的 wire 面（前端 refetch 链的引擎半边）：开轮发布的 Updated 携带
/// `turn_engaged=false`（SEED 不占用，design D3 词表），首个内容帧触发的
/// SEED→WORKING 迁移发布的 Updated 携带 `turn_engaged=true`——静默窗内
/// WS 驱动的 refetch 读到的就是这份事实（提交控件据此呈停止形态）。
#[tokio::test]
async fn seed_to_working_transition_publishes_updated_with_turn_engaged() {
    let map = SessionMap::new();
    let key = web_key("wire");
    let (router, _out_rx) = DispatchHandle::new(map);
    let mut events = router.subscribe_session_events();

    router
        .map
        .insert(key.clone(), Mapping::active("s-wire"))
        .await
        .unwrap();
    router
        .seed_card("s-wire".to_string(), "run".into())
        .await;
    router
        .dispatch_acp_event(AcpEvent::TextDelta {
            session_id: "s-wire".into(),
            delta: "first frame".into(),
        })
        .await;

    // 收集 Updated 序列：seed 后（SEED）= 不占用；内容帧后（WORKING）= 占用。
    let mut engaged_timeline: Vec<bool> = Vec::new();
    while let Ok(ev) = events.try_recv() {
        if let SessionEvent::Updated { session } = ev {
            if session.key == key.reference {
                engaged_timeline.push(session.turn_engaged);
            }
        }
    }
    assert_eq!(
        engaged_timeline,
        vec![false, true],
        "the Updated wire must carry the engagement fact at both milestones (SEED then WORKING)"
    );
}

/// 终态 Error 拆除会话时，看门狗的时钟与泊车登记一并清空（复用 id 不继承
/// stale 事实）。
#[tokio::test]
async fn terminal_error_drops_the_stall_facts() {
    let map = SessionMap::new();
    let key = web_key("teardown");
    let (router, _out_rx) = DispatchHandle::new(map);
    router.set_turn_stall_timeout(600);

    seed_working_session(&router, &key, "s-dead").await;
    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s-dead".into(),
            message: "boom".into(),
            terminal: true,
        })
        .await;

    let info = router.session_info_for(&key).await;
    assert!(info.is_none(), "terminal error tears the mapping down");
    // 登记表清空：扫描无事实可看（不 panic、不复活 stale 时钟）。
    assert!(router.force_settle_stalled_turns().await.is_empty());
}

/// 辅助：CardStateMap 的相位快照直读（测试可见性）。
trait StatusProbe {
    async fn card_state_status_emoji(&self, session_id: &str) -> Option<String>;
}

impl StatusProbe for DispatchHandle {
    async fn card_state_status_emoji(&self, session_id: &str) -> Option<String> {
        // engine_test 系列经 dispatch 驱动；这里用公开的 card_state_snapshot。
        self.card_state_snapshot()
            .await
            .remove(session_id)
            .map(|st| st.status_emoji)
    }
}

// 引用 ChannelEvent 以免未使用告警（相位驱动走 dispatch_acp_event）。
#[allow(unused)]
fn _touch(_: Option<ChannelEvent>) {}
