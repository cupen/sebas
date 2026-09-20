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
                cmd: sebas_acp::claude::session::AcpCommand::ContinueSession { prompt, .. },
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
            Ok(SessionEvent::TurnStalled {
                key: k, released, ..
            }) => {
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
            Ok(SessionEvent::TurnStalled {
                key: k, released, ..
            }) if k == key.reference => {
                notified = Some(released);
                break;
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    assert_eq!(
        notified,
        Some(1),
        "the notice must fire for a SEED stall too"
    );
}

/// 状态迁移的 wire 面（前端 refetch 链的引擎半边）：开轮发布的 Updated 携带
/// `turn_engaged=true`（round4 2.2：带 prompt 的 SEED = 接收回执相位，在飞
/// ——提交控件据此在「已收到」窗口呈停止形态），首个内容帧触发的
/// SEED→WORKING 迁移发布的 Updated 继续携带 `turn_engaged=true`——静默窗内
/// WS 驱动的 refetch 读到的就是这份事实。
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
    router.seed_card("s-wire".to_string(), "run".into()).await;
    router
        .dispatch_acp_event(AcpEvent::TextDelta {
            session_id: "s-wire".into(),
            delta: "first frame".into(),
        })
        .await;

    // 收集 Updated 序列：seed 后（接收回执 SEED）= 占用；内容帧后（WORKING）
    // = 占用。
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
        vec![true, true],
        "the Updated wire must carry the engagement fact at both milestones (receipt then WORKING)"
    );
}

/// 终态 Error 拆除会话时，看门狗的时钟与泊车登记一并清空（复用 id 不继承
/// stale 事实）。round4 1.2：活跃绑定退役为 Dormant 记录（行保留），但
/// stall 事实（时钟 + 泊车登记）照旧清空——记录保留 ≠ 运行时状态保留。
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

    // 活跃绑定已退役：行还在（dormant）但不再构成在飞回合。
    let info = router
        .session_info_for(&key)
        .await
        .expect("the record survives teardown");
    assert_eq!(info.status, "dormant");
    assert!(!info.turn_engaged, "a retired record occupies no turn");
    // 登记表清空：扫描无事实可看（不 panic、不复活 stale 时钟）。
    assert!(router.force_settle_stalled_turns().await.is_empty());
}

/// 辅助：CardStateMap 的相位快照直读（测试可见性）。
trait StatusProbe {
    async fn card_state_status_emoji(&self, session_id: &str) -> Option<String>;
}

// ── fix-webui-qa-defects 3.1/3.2：占位不武装看门狗（design D2）──────────────

/// 3.1 场景 a（占位旗标仍在）：0-turn 占位（awaiting_first_prompt = true）
/// 卡片经事件 lazy seed 落在 SEED、时钟被拨到远超阈值，看门狗也不收尾、
/// 不写合成错误条目——占位不构成在飞回合。
#[tokio::test]
async fn idle_placeholder_with_flag_is_never_stall_settled() {
    let map = SessionMap::new();
    let key = web_key("ghost-flag");
    let (router, _out_rx) = DispatchHandle::new(map);
    router.set_turn_stall_timeout(600);

    // 占位：awaiting_first_prompt = true 的 Spawning 映射 + 一个挂在占位
    // 身上的 SEED 卡与 stale 时钟（QA 复现形态：握手事件 lazy seed）。
    router
        .map
        .begin_spawn_with(key.clone(), None, None, None, true)
        .await
        .unwrap();
    router
        .map
        .set_project_dir(&key, Some("/proj".into()))
        .await;
    let sid = "sid-placeholder";
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: sid.into(),
            usage: sebas_acp::TurnUsage {
                model: None,
                input_tokens: None,
                output_tokens: None,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        })
        .await;
    assert_eq!(
        router.card_state_status_emoji(sid).await.as_deref(),
        Some(phase::SEED),
        "setup: lazy-seeded card sits in SEED"
    );
    router
        .stall_registry()
        .rewind_last_event_for_test(sid, 3600)
        .await;

    let settled = router.force_settle_stalled_turns().await;
    assert!(
        settled.is_empty(),
        "an awaiting-first-prompt placeholder must never be force-settled"
    );
    // transcript 干净：无「回合停滞被强制收尾」合成错误。
    let turns = router.session_turns(&key, 0).await.unwrap_or_default();
    assert!(
        !turns
            .iter()
            .any(|t| t.element_type == "error" && t.content.contains("回合停滞")),
        "no synthetic stall error may be injected into a placeholder: {turns:?}"
    );
}

/// 3.1 场景 b（占位幽灵回合的实体——激活后无轮）：占位经聚焦拉起（旗标
/// 已被激活消费、映射 Active、卡片 lazy seed 于 SEED、transcript 零条目）
/// 沉默超阈值，看门狗同样不收尾——空 transcript = 从未开轮。
#[tokio::test]
async fn activated_never_prompted_session_is_not_stall_settled() {
    let map = SessionMap::new();
    let key = web_key("ghost-activated");
    let (router, _out_rx) = DispatchHandle::new(map);
    router.set_turn_stall_timeout(600);

    // 激活完成后的形态：Active 映射 + lazy-seeded SEED 卡 + stale 时钟、
    // 无任何 transcript 条目（激活 spawn 不带 prompt，seed_card 未跑）。
    router
        .map
        .insert(key.clone(), Mapping::active("sid-idle"))
        .await
        .unwrap();
    router
        .dispatch_acp_event(AcpEvent::UsageUpdate {
            session_id: "sid-idle".into(),
            usage: sebas_acp::TurnUsage {
                model: None,
                input_tokens: None,
                output_tokens: None,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        })
        .await;
    router
        .stall_registry()
        .rewind_last_event_for_test("sid-idle", 3600)
        .await;

    let settled = router.force_settle_stalled_turns().await;
    assert!(
        settled.is_empty(),
        "a session that never started a turn must not be force-settled (ghost-turn fix)"
    );
    assert!(
        !router
            .session_turns(&key, 0)
            .await
            .unwrap()
            .iter()
            .any(|t| t.element_type == "error"),
        "no synthetic error may appear in an idle session's transcript"
    );
}

/// 3.2：首条消息使占位转入真实 spawn 并开轮（activate + seed_card 落下
/// prompt 条目）后，看门狗对同一会话恢复生效——占位豁免绝不外溢到真实回合。
#[tokio::test]
async fn first_message_turn_is_still_guarded_after_the_placeholder() {
    let map = SessionMap::new();
    let key = web_key("real-turn");
    let (router, _out_rx) = DispatchHandle::new(map);
    router.set_turn_stall_timeout(600);
    let sid = "sid-real";

    // 占位：awaiting_first_prompt = true。
    router
        .map
        .begin_spawn_with(key.clone(), None, None, None, true)
        .await
        .unwrap();
    // 首条消息消费占位旗标（SpawnNew），随后 spawn 完成、映射翻 Active，
    // seed_card 开轮（prompt 条目落 transcript）——真实回合成立。
    let route = router
        .map
        .route_text(key.clone(), "the first real prompt".into())
        .await
        .unwrap();
    assert!(matches!(route, sebas_dispatch::state::TextRoute::SpawnNew));
    router.activate(&key, sid.to_string(), None, None).await;
    router
        .seed_card(sid.to_string(), "the first real prompt".into())
        .await;
    // 真实回合的开轮事实：prompt 条目已落 transcript（占位的 transcript
    // 恒空——这是看门狗区分二者的引擎事实）。
    let turns = router.session_turns(&key, 0).await.unwrap();
    assert!(
        turns.iter().any(|t| t.kind == "prompt"),
        "setup: the real turn's prompt entry must be in the transcript: {turns:?}"
    );
    router
        .stall_registry()
        .rewind_last_event_for_test(sid, 3600)
        .await;

    let settled = router.force_settle_stalled_turns().await;
    assert_eq!(
        settled,
        vec![(key.clone(), 0)],
        "a real turn on the formerly-placeholder session is guarded as today"
    );
    // 收尾条目带 stall 分类（5.1，design D5）。
    let turns = router.session_turns(&key, 0).await.unwrap();
    let stall_entry = turns
        .iter()
        .find(|t| t.element_type == "error" && t.content.contains("回合停滞"))
        .expect("the stall entry must be in the transcript");
    assert_eq!(stall_entry.failure_class.as_deref(), Some("stall"));
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

/// （review 3c 补口，session-unread-badge delta「parked-approval entry/exit
/// emits a frame」）泊车登记与批复解除都是生命周期 flip：登记即刻广播
/// Updated（SessionInfo.parked_approvals=1），批复出站（emit 单点）即刻再
/// 广播（parked_approvals 归 0）。呈现层据此把本地会话投影成 waiting /
/// 翻回原相位，rail 圆点与徽标不等轮询。
#[tokio::test]
async fn parked_entry_and_exit_each_publish_updated_with_parked_count() {
    let map = SessionMap::new();
    let key = web_key("parked-flip");
    let (router, _out_rx) = DispatchHandle::new(map);
    let mut events = router.subscribe_session_events();

    seed_working_session(&router, &key, "s-parked-flip").await;

    // 排空 seed/TextDelta 阶段的既有 Updated（parked=0 的旧帧）——后续断言
    // 只认泊车登记/解除引发的帧。
    while events.try_recv().is_ok() {}

    // 泊车登记 → 一条 Updated，且 parked_approvals = 1。
    router
        .dispatch_acp_event(AcpEvent::PermissionRequest {
            session_id: "s-parked-flip".into(),
            request_id: "req-flip-1".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"command": "rm -rf /"}),
        })
        .await;
    let mut saw_parked_entry = false;
    while let Ok(ev) = events.try_recv() {
        if let SessionEvent::Updated { session } = ev {
            if session.key == key.reference {
                assert_eq!(
                    session.parked_approvals, 1,
                    "the entry flip must carry the parked count"
                );
                saw_parked_entry = true;
            }
        }
    }
    assert!(saw_parked_entry, "permission entry must emit an Updated frame");

    // 批复出站（emit 单点漏斗）→ 一条 Updated，parked_approvals 归 0。
    router
        .emit(Out::SendAcp {
            session_id: "s-parked-flip".into(),
            cmd: sebas_acp::claude::session::AcpCommand::PermissionReply {
                session_id: "s-parked-flip".into(),
                request_id: "req-flip-1".into(),
                decision: sebas_acp::Decision::AllowOnce,
            },
        })
        .await;
    let mut saw_parked_exit = false;
    while let Ok(ev) = events.try_recv() {
        if let SessionEvent::Updated { session } = ev {
            if session.key == key.reference {
                assert_eq!(
                    session.parked_approvals, 0,
                    "the exit flip must carry the cleared count"
                );
                saw_parked_exit = true;
            }
        }
    }
    assert!(
        saw_parked_exit,
        "permission resolution must emit an Updated frame"
    );
}
