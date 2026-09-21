//! 零输出回合的合成提示落点（close-acceptance-blind-spots 4.1，design D3）。
//!
//! spec「Turn completing without visible output appends a notice」：
//! - 空回合（无任何可见输出）正常结束 → 投影追加一条合成 `notice` 条目，
//!   回合在时间线上可见（`/code-review` 零回音事故的落点）；
//! - 正常回合（正文/thinking/工具/错误任一）→ 绝不追加；
//! - 从未开轮的会话（占位/激活幽灵回合）收到游离 Finished → 不注入
//!   （沿 fix-webui-qa-defects 3.1「占位不构成在飞回合」语义）。
//!
//! 纯引擎路径（不拉子进程）：映射直插 + seed_card / dispatch_acp_event
//! 驱动，与 turn_stall_test.rs 同一 harness。

use sebas_acp::claude::session::AcpEvent;
use sebas_channels::ChannelKey;
use sebas_dispatch::engine::{DispatchHandle, TurnStreamEvent};
use sebas_dispatch::state::{Mapping, SessionMap};

fn web_key(tag: &str) -> ChannelKey {
    ChannelKey::new("web", format!("zero-out-{tag}"))
}

/// 造一个活跃会话并开轮（seed_card 落下 prompt 条目 = 真实回合成立）。
async fn seed_turn(router: &DispatchHandle, key: &ChannelKey, sid: &str, prompt: &str) {
    router
        .map
        .insert(key.clone(), Mapping::active(sid))
        .await
        .unwrap();
    router.seed_card(sid.to_string(), prompt.into()).await;
}

fn notice_entries(turns: &[sebas_dispatch::TurnEntry]) -> Vec<&sebas_dispatch::TurnEntry> {
    turns.iter().filter(|t| t.element_type == "notice").collect()
}

/// spec 场景「空回合有落点」：回合零输出正常结束 → 投影恰好一条 notice
/// 条目（prompt 之后），文案说明「回合已结束且无输出」；notice 经 turn 流
/// 广播（时间线实时可见），重复 Finished 不产生第二条。
#[tokio::test]
async fn empty_turn_appends_exactly_one_notice() {
    let map = SessionMap::new();
    let key = web_key("empty");
    let (router, _out_rx) = DispatchHandle::new(map);
    let mut turn_stream = router.subscribe_turn_events();
    seed_turn(&router, &key, "s-empty", "say nothing").await;

    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-empty".into(),
        })
        .await;

    let turns = router.session_turns(&key, 0).await.unwrap();
    let notices = notice_entries(&turns);
    assert_eq!(
        notices.len(),
        1,
        "an empty turn must land exactly one notice entry: {turns:?}"
    );
    assert_eq!(notices[0].kind, "content", "the notice is core-produced");
    assert!(
        notices[0].content.contains("回合已结束且无输出"),
        "the notice must say the turn ended without output: {}",
        notices[0].content
    );
    // 时间线形态：prompt（操作者提交）在前，notice 在后——回合可见地收尾。
    assert_eq!(turns.len(), 2, "prompt + notice, nothing else: {turns:?}");
    assert_eq!(turns[0].kind, "prompt");
    assert_eq!(turns[1].element_type, "notice");

    // turn 流同样携带 notice（实时时间线可见，不只落快照）。
    let mut streamed_notice = false;
    while let Ok(ev) = turn_stream.try_recv() {
        let TurnStreamEvent { entries, .. } = ev;
        if entries.iter().any(|e| e.element_type == "notice") {
            streamed_notice = true;
        }
    }
    assert!(
        streamed_notice,
        "the notice must ride the turn stream (live timeline)"
    );

    // 防御：异常的重复 Finished 不重复追加。
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-empty".into(),
        })
        .await;
    let turns = router.session_turns(&key, 0).await.unwrap();
    assert_eq!(
        notice_entries(&turns).len(),
        1,
        "a repeated Finished must not append a second notice: {turns:?}"
    );
}

/// spec 场景「正常回合不受影响」（正文）：有正文条目的回合结束 → 不追加。
#[tokio::test]
async fn text_turn_does_not_append_notice() {
    let map = SessionMap::new();
    let key = web_key("text");
    let (router, _out_rx) = DispatchHandle::new(map);
    seed_turn(&router, &key, "s-text", "hello").await;
    router
        .dispatch_acp_event(AcpEvent::TextDelta {
            session_id: "s-text".into(),
            delta: "hi there".into(),
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-text".into(),
        })
        .await;

    let turns = router.session_turns(&key, 0).await.unwrap();
    assert!(
        notice_entries(&turns).is_empty(),
        "a turn with text output must never get a notice: {turns:?}"
    );
}

/// 「正常回合不受影响」（thinking / 工具）：过程条目同样算可见输出。
#[tokio::test]
async fn process_only_turn_does_not_append_notice() {
    let map = SessionMap::new();
    let key = web_key("process");
    let (router, _out_rx) = DispatchHandle::new(map);

    // thinking-only（默认 ThinkingDisplay::Show，thinking 条目落账）。
    seed_turn(&router, &key, "s-think", "think silently").await;
    router
        .dispatch_acp_event(AcpEvent::ThinkingDelta {
            session_id: "s-think".into(),
            delta: "hmm".into(),
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-think".into(),
        })
        .await;

    // tool-only。
    let tool_key = web_key("tool");
    seed_turn(&router, &tool_key, "s-tool", "use a tool").await;
    router
        .dispatch_acp_event(AcpEvent::ToolStart {
            session_id: "s-tool".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"command": "ls"}),
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-tool".into(),
        })
        .await;

    for (k, sid, what) in [
        (&key, "s-think", "thinking"),
        (&tool_key, "s-tool", "tool"),
    ] {
        let turns = router.session_turns(k, 0).await.unwrap();
        assert!(
            notice_entries(&turns).is_empty(),
            "a {what}-only turn counts as visible output and must not get a notice: {turns:?}"
        );
        assert!(
            turns.iter().any(|t| t.kind == "content"),
            "the {what} entry must be in the transcript"
        );
        let _ = sid;
    }
}

/// 「正常回合不受影响」（错误）：拒绝回合的 Error + Finished 配对——error
/// 条目已落账，零输出检测不得再叠一条 notice（refusal 语义归错误条目）。
#[tokio::test]
async fn refusal_error_turn_does_not_append_notice() {
    let map = SessionMap::new();
    let key = web_key("refusal");
    let (router, _out_rx) = DispatchHandle::new(map);
    seed_turn(&router, &key, "s-refusal", "do the thing").await;
    // 驱动的拒绝映射：非终态 Error（拒绝正文）+ Finished 配对。
    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s-refusal".into(),
            message: "I cannot help with that".into(),
            terminal: false,
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-refusal".into(),
        })
        .await;

    let turns = router.session_turns(&key, 0).await.unwrap();
    assert!(
        turns.iter()
            .any(|t| t.element_type == "error" && t.content.contains("I cannot help")),
        "the refusal error entry must be present: {turns:?}"
    );
    assert!(
        notice_entries(&turns).is_empty(),
        "a refused turn already has a visible error entry; no notice on top: {turns:?}"
    );
}

/// 从未开轮的会话（无 prompt 条目：占位/激活幽灵回合）收到游离 Finished →
/// 不注入任何合成条目（占位幽灵回合不得被伪造出「回合」痕迹）。
#[tokio::test]
async fn finished_without_a_seeded_turn_appends_nothing() {
    let map = SessionMap::new();
    let key = web_key("ghost");
    let (router, _out_rx) = DispatchHandle::new(map);
    router
        .map
        .insert(key.clone(), Mapping::active("sid-ghost"))
        .await
        .unwrap();

    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "sid-ghost".into(),
        })
        .await;

    let turns = router.session_turns(&key, 0).await.unwrap();
    assert!(
        turns.is_empty(),
        "a session that never started a turn must stay clean: {turns:?}"
    );
}
