use feishu::cards::{apply_event_to_card, render_permission_card, render_root_card, CardConfig, CardElement};
use acp_claude::session::AcpEvent;

#[test]
fn root_card_initial_snapshot() {
    let card = render_root_card("重构 src/foo.rs", "msg_1", "👀");
    insta::assert_yaml_snapshot!(card);
}

#[test]
fn root_card_after_text_delta_snapshot() {
    let mut card = render_root_card("重构 src/foo.rs", "msg_1", "🚧");
    card.push_text("我会先看一下 foo.rs 的结构。");
    insta::assert_yaml_snapshot!(card);
}

#[test]
fn permission_card_snapshot() {
    let card = render_permission_card("s1", "r1", "Bash", &serde_json::json!({"cmd": "rm -rf"}));
    insta::assert_yaml_snapshot!(card);
}

#[test]
fn card_config_defaults() {
    use feishu::cards::CardConfig;
    let c = CardConfig::default();
    assert_eq!(c.theme_color, "blue");
    assert_eq!(c.max_user_text_chars, 4000);
    assert_eq!(c.max_tool_output_chars, 2000);
    assert!(c.fold_long_output);
}

#[test]
fn card_config_from_toml() {
    use feishu::cards::CardConfig;
    let toml = r#"
theme_color = "orange"
max_user_text_chars = 100
max_tool_output_chars = 50
fold_long_output = false
"#;
    let c: CardConfig = toml::from_str(toml).unwrap();
    assert_eq!(c.theme_color, "orange");
    assert_eq!(c.max_user_text_chars, 100);
    assert_eq!(c.max_tool_output_chars, 50);
    assert!(!c.fold_long_output);
}

fn cfg() -> CardConfig {
    CardConfig::default()
}

fn cfg_small() -> CardConfig {
    CardConfig {
        max_user_text_chars: 10,
        max_tool_output_chars: 5,
        fold_long_output: true,
        ..cfg()
    }
}

#[test]
fn append_text_delta() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::TextDelta { session_id: "s".into(), delta: "hi".into() },
        &cfg(),
    );
    assert_eq!(body.len(), 1);
    match &body[0] {
        CardElement::Markdown { content } => assert_eq!(content, "hi"),
        other => panic!("expected Markdown, got {other:?}"),
    }
}

#[test]
fn append_revives_thinking_toolend_toolprogress() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta { session_id: "s".into(), delta: "thinking".into() },
        &cfg(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolProgress { session_id: "s".into(), tool_name: "Bash".into(), progress: "in_progress".into() },
        &cfg(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolEnd { session_id: "s".into(), tool_name: "Bash".into(), result: "ok".into() },
        &cfg(),
    );
    // ThinkingDelta -> Div; ToolProgress -> Div; ToolEnd -> Div（各 1 个元素）
    assert_eq!(body.len(), 3);
    assert!(matches!(body[0], CardElement::Div { .. }));
    assert!(matches!(body[1], CardElement::Div { .. }));
    assert!(matches!(body[2], CardElement::Div { .. }));
}

#[test]
fn tool_start_emits_hr_then_markdown() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart { session_id: "s".into(), tool_name: "Bash".into(), args: serde_json::json!({"cmd":"ls"}) },
        &cfg(),
    );
    assert!(matches!(body[0], CardElement::Hr));
    assert!(matches!(body[1], CardElement::Markdown { .. }));
}

#[test]
fn long_text_is_truncated_with_grey_note() {
    let mut body = vec![];
    let big = "a".repeat(50);
    apply_event_to_card(
        &mut body,
        &AcpEvent::TextDelta { session_id: "s".into(), delta: big.clone() },
        &cfg_small(),
    );
    // TextDelta 截断到 10 + 灰注（已折叠 40 字），共 2 个元素。
    assert_eq!(body.len(), 2);
    match &body[0] {
        CardElement::Markdown { content } => {
            assert_eq!(content.chars().count(), 10);
        }
        other => panic!("expected Markdown, got {other:?}"),
    }
    match &body[1] {
        CardElement::Div { text } => assert!(text.content.contains("已折叠 40 字")),
        other => panic!("expected Div note, got {other:?}"),
    }
}

#[test]
fn fold_disabled_skips_truncation() {
    let mut body = vec![];
    let big = "a".repeat(50);
    let c = CardConfig { fold_long_output: false, ..cfg_small() };
    apply_event_to_card(
        &mut body,
        &AcpEvent::TextDelta { session_id: "s".into(), delta: big },
        &c,
    );
    // 不截断：单元素，全文保留。
    assert_eq!(body.len(), 1);
    match &body[0] {
        CardElement::Markdown { content } => assert_eq!(content.chars().count(), 50),
        other => panic!("expected Markdown, got {other:?}"),
    }
}

#[test]
fn long_toolend_result_truncated() {
    let mut body = vec![];
    let big = "x".repeat(20);
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolEnd { session_id: "s".into(), tool_name: "Bash".into(), result: big },
        &cfg_small(),
    );
    // ToolEnd.result 截断到 5 + 灰注，共 2 个元素。
    assert_eq!(body.len(), 2);
}

#[test]
fn total_budget_drops_oldest() {
    // 总量 > 24000 -> 丢旧。用 max_user_text_chars=4000（default），塞 7 段 4000 字 = 28000 -> 丢到 ≤24000（丢 1 段 -> 24000）。
    let mut body = vec![];
    let c = cfg(); // max_user_text_chars=4000, total budget 24000
    for _ in 0..7 {
        apply_event_to_card(
            &mut body,
            &AcpEvent::TextDelta { session_id: "s".into(), delta: "a".repeat(4000) },
            &c,
        );
    }
    // 7*4000=28000 > 24000 -> 丢最旧 1 段 -> 6 段 *4000 = 24000 (==budget, 不再丢).
    assert_eq!(body.len(), 6);
}

#[test]
fn total_budget_drops_hr_with_following_element() {
    // 最旧是 Hr -> 连后一个一起丢。
    let mut body = vec![];
    let c = cfg();
    // 先 push 一个 Hr + 一个 text，再 push 大量 text 触发总量。
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart { session_id: "s".into(), tool_name: "Bash".into(), args: serde_json::json!({}) },
        &c,
    ); // body = [Hr, Markdown]
    for _ in 0..7 {
        apply_event_to_card(
            &mut body,
            &AcpEvent::TextDelta { session_id: "s".into(), delta: "a".repeat(4000) },
            &c,
        );
    } // body = [Hr, Markdown, M, M, M, M, M, M, M] -> Hr 最旧
    // 总量超 24000 -> 丢 Hr + 其后 1 个 Markdown（共 2 个），剩余 7-1=6 段 text + 原 Markdown? 需算:
    //   元素: [Hr, Markdown(ToolStart的), M, M, M, M, M, M, M] = 1 Hr + 8 Markdown
    //   字符: 8*4000 = 32000 -> 丢 Hr+第1个M -> 7*4000=28000 -> 继续 -> 丢第2个M -> 6*4000=24000 -> 停.
    //   但丢 Hr 时连后一个 -> 第一次丢 [Hr, Markdown(ToolStart)] -> 剩 7 M = 28000 -> 再丢 1 M -> 6 M = 24000.
    //   最终 body.len() = 6 (6 个 Markdown).
    assert_eq!(body.len(), 6);
    // 最旧的 Hr 已被连带丢掉.
    assert!(matches!(body[0], CardElement::Markdown { .. }));
}

#[test]
fn permission_request_is_noop_for_body() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::PermissionRequest { session_id: "s".into(), request_id: "r".into(), tool_name: "Bash".into(), args: serde_json::json!({}) },
        &cfg(),
    );
    assert!(body.is_empty(), "PermissionRequest 不累积进 root body");
}

#[test]
fn finished_and_error_append_markdown() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::Finished { session_id: "s".into() },
        &cfg(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::Error { session_id: "s".into(), message: "boom".into(), terminal: false },
        &cfg(),
    );
    assert_eq!(body.len(), 2);
    match &body[0] {
        CardElement::Markdown { content } => assert_eq!(content, "✅ 完成"),
        other => panic!("expected Finished Markdown, got {other:?}"),
    }
    match &body[1] {
        CardElement::Markdown { content } => assert_eq!(content, "❌ boom"),
        other => panic!("expected Error Markdown, got {other:?}"),
    }
}
