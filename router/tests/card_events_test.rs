use acp_claude::session::AcpEvent;
use feishu::cards::{CardConfig, CardElement};
use router::card_events::apply_event_to_card;

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

/// fold 模式下跑工具链路的配置：args 不截断（上限足够），结果软上限很小。
fn cfg_fold_tool() -> CardConfig {
    CardConfig {
        max_user_text_chars: 1000,
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
        &AcpEvent::TextDelta {
            session_id: "s".into(),
            delta: "hi".into(),
        },
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
        &AcpEvent::ThinkingDelta {
            session_id: "s".into(),
            delta: "thinking".into(),
        },
        &cfg(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolProgress {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            progress: "in_progress".into(),
        },
        &cfg(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolEnd {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            result: "ok".into(),
        },
        &cfg(),
    );
    // ThinkingDelta -> Hr + Div；ToolProgress（无 tool 面板）-> 独立 Div；
    // ToolEnd（默认 max_tool_output_chars=0 且无面板可归属）-> 静默。
    assert_eq!(body.len(), 3);
    assert!(matches!(body[0], CardElement::Hr));
    assert!(matches!(body[1], CardElement::Div { .. }));
    assert!(matches!(body[2], CardElement::Div { .. }));
}

#[test]
fn tool_start_folds_into_collapsible_panel() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"cmd":"ls"}),
        },
        &cfg(),
    );
    // 默认折叠：单个 collapsible_panel，标题 📖 Bash，args 在面板内。
    assert_eq!(body.len(), 1);
    match &body[0] {
        CardElement::CollapsiblePanel(panel) => {
            assert!(!panel.expanded, "默认折叠");
            assert_eq!(panel.header.title.content, "📖 Bash");
            assert!(matches!(panel.elements[0], CardElement::Markdown { .. }));
        }
        other => panic!("expected CollapsiblePanel, got {other:?}"),
    }
}

#[test]
fn tool_start_fold_disabled_emits_hr_then_markdown() {
    let mut body = vec![];
    let c = CardConfig {
        fold_long_output: false,
        ..cfg()
    };
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"cmd":"ls"}),
        },
        &c,
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
        &AcpEvent::TextDelta {
            session_id: "s".into(),
            delta: big.clone(),
        },
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
    let c = CardConfig {
        fold_long_output: false,
        ..cfg_small()
    };
    apply_event_to_card(
        &mut body,
        &AcpEvent::TextDelta {
            session_id: "s".into(),
            delta: big,
        },
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
fn tool_lifecycle_folds_into_single_panel() {
    // ToolStart + ToolProgress + ToolEnd 全部收进同一个折叠面板。
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"cmd": "ls"}),
        },
        &cfg_fold_tool(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolProgress {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            progress: "in_progress".into(),
        },
        &cfg_fold_tool(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolEnd {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            result: "x".repeat(20),
        },
        &cfg_fold_tool(),
    );
    // 一个工具 = 一行面板；标题随生命周期变成 ✓ Bash。
    assert_eq!(body.len(), 1);
    match &body[0] {
        CardElement::CollapsiblePanel(panel) => {
            assert_eq!(panel.header.title.content, "✓ Bash");
            // [args markdown, 进度灰注, 结果 markdown（超过软上限 5，全文保留 20 字）]
            assert_eq!(panel.elements.len(), 3);
            match &panel.elements[0] {
                CardElement::Markdown { content } => assert!(content.contains("```json")),
                other => panic!("expected args Markdown, got {other:?}"),
            }
            match &panel.elements[1] {
                CardElement::Div { text } => assert!(text.content.contains("in_progress")),
                other => panic!("expected progress Div, got {other:?}"),
            }
            match &panel.elements[2] {
                CardElement::Markdown { content } => assert_eq!(content.chars().count(), 20),
                other => panic!("expected result Markdown, got {other:?}"),
            }
        }
        other => panic!("expected CollapsiblePanel, got {other:?}"),
    }
}

#[test]
fn tool_end_zero_suppresses_result_output() {
    // 默认 max_tool_output_chars=0：结果内容不输出，面板只保留 args + 完成标记。
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"cmd": "ls"}),
        },
        &cfg(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolEnd {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            result: "whatever".into(),
        },
        &cfg(),
    );
    assert_eq!(body.len(), 1);
    match &body[0] {
        CardElement::CollapsiblePanel(panel) => {
            assert_eq!(panel.header.title.content, "✓ Bash");
            assert_eq!(panel.elements.len(), 1, "只有 args，没有结果内容");
        }
        other => panic!("expected CollapsiblePanel, got {other:?}"),
    }
    let s = serde_json::to_string(&body).unwrap();
    assert!(!s.contains("whatever"), "结果内容必须被屏蔽");
}

#[test]
fn tool_end_hard_limit_truncates_inside_panel() {
    let mut body = vec![];
    let big = "y".repeat(10240 + 10);
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({}),
        },
        &cfg_fold_tool(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolEnd {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            result: big,
        },
        &cfg_fold_tool(),
    );
    match &body[0] {
        CardElement::CollapsiblePanel(panel) => {
            // [args markdown, 硬上限截断后的结果 markdown, 截断灰注]
            assert_eq!(panel.elements.len(), 3);
            match &panel.elements[1] {
                CardElement::Markdown { content } => assert_eq!(content.chars().count(), 10240),
                other => panic!("expected Markdown, got {other:?}"),
            }
            match &panel.elements[2] {
                CardElement::Div { text } => assert!(text.content.contains("已截断 10 字")),
                other => panic!("expected truncation note, got {other:?}"),
            }
        }
        other => panic!("expected CollapsiblePanel, got {other:?}"),
    }
}

#[test]
fn tool_end_fold_disabled_shows_full_content_inline() {
    let mut body = vec![];
    let big = "z".repeat(20);
    let c = CardConfig {
        fold_long_output: false,
        ..cfg_small()
    };
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolEnd {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            result: big,
        },
        &c,
    );
    // 不折叠：单条灰注内联，全文保留（仍受 10240 硬上限保护）。
    assert_eq!(body.len(), 1);
    match &body[0] {
        CardElement::Div { text } => assert!(text.content.contains(&"z".repeat(20))),
        other => panic!("expected Div note, got {other:?}"),
    }
}

#[test]
fn total_budget_drops_oldest() {
    // 总量 > 24000 -> 丢旧。用 max_user_text_chars=4000（default），塞 7 段 4000 字 = 28000 -> 丢到 ≤24000（丢 1 段 -> 24000）。
    let mut body = vec![];
    let c = cfg(); // max_user_text_chars=4000, total budget 24000
    for _ in 0..7 {
        apply_event_to_card(
            &mut body,
            &AcpEvent::TextDelta {
                session_id: "s".into(),
                delta: "a".repeat(4000),
            },
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
    // fold=false 才产生 Hr（fold=true 时 ToolStart 是折叠面板）。
    let c = CardConfig {
        fold_long_output: false,
        ..cfg()
    };
    // 先 push 一个 Hr + 一个 text，再 push 大量 text 触发总量。
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({}),
        },
        &c,
    ); // body = [Hr, Markdown]
    for _ in 0..7 {
        apply_event_to_card(
            &mut body,
            &AcpEvent::TextDelta {
                session_id: "s".into(),
                delta: "a".repeat(4000),
            },
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
        &AcpEvent::PermissionRequest {
            session_id: "s".into(),
            request_id: "r".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({}),
        },
        &cfg(),
    );
    assert!(body.is_empty(), "PermissionRequest 不累积进 root body");
}

#[test]
fn finished_and_error_append_markdown() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::Finished {
            session_id: "s".into(),
        },
        &cfg(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::Error {
            session_id: "s".into(),
            message: "boom".into(),
            terminal: false,
        },
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

#[test]
fn tool_start_renders_args_in_code_fence() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"command": "ls /tmp"}),
        },
        &cfg(),
    );
    let s = serde_json::to_string(&body).unwrap();
    assert!(s.contains("```json"), "ToolStart args must be fenced");
    assert!(
        s.contains("\\\"command\\\""),
        "ToolStart args must contain escaped command key"
    );
    assert!(
        s.contains("ls /tmp"),
        "ToolStart args must contain the command value"
    );
}
