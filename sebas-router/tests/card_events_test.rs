use sebas_acp::claude::session::AcpEvent;
use sebas_feishu::cards::{CardConfig, CardElement, ThinkingDisplay};
use sebas_router::card_events::apply_event_to_card;

fn cfg() -> CardConfig {
    CardConfig::default()
}

fn cfg_show() -> CardConfig {
    CardConfig::default() // thinking = Show
}

fn cfg_hide() -> CardConfig {
    CardConfig {
        thinking: ThinkingDisplay::Hide,
        ..CardConfig::default()
    }
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
fn thinking_hide_drops_delta() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta {
            session_id: "s".into(),
            delta: "hidden".into(),
        },
        &cfg_hide(),
    );
    assert!(body.is_empty(), "hide 模式必须完全丢弃 ThinkingDelta");
}

#[test]
fn thinking_show_aggregates_adjacent_deltas() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta {
            session_id: "s".into(),
            delta: "A".into(),
        },
        &cfg_show(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta {
            session_id: "s".into(),
            delta: "B".into(),
        },
        &cfg_show(),
    );
    // 收在父面板内：body[0] 是父面板，其内子面板内容 "A\nB"。
    assert_eq!(body.len(), 1);
    let CardElement::CollapsiblePanel(parent) = &body[0] else {
        panic!("expected parent CollapsiblePanel, got {:?}", &body[0]);
    };
    assert_eq!(parent.elements.len(), 1);
    let CardElement::CollapsiblePanel(panel) = &parent.elements[0] else {
        panic!(
            "expected thinking CollapsiblePanel, got {:?}",
            &parent.elements[0]
        );
    };
    assert_eq!(panel.elements.len(), 1);
    match &panel.elements[0] {
        CardElement::Markdown { content } => assert_eq!(content, "A\nB"),
        other => panic!("expected Markdown, got {other:?}"),
    }
}

#[test]
fn thinking_show_starts_new_panel_on_non_thinking_event() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta {
            session_id: "s".into(),
            delta: "first".into(),
        },
        &cfg_show(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::TextDelta {
            session_id: "s".into(),
            delta: "interlude".into(),
        },
        &cfg_show(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta {
            session_id: "s".into(),
            delta: "second".into(),
        },
        &cfg_show(),
    );
    // 2 个元素：父面板（内含 1 个 thinking 子面板，两段 delta 已合并）, markdown("interlude")。
    assert_eq!(body.len(), 2);
    match &body[0] {
        CardElement::CollapsiblePanel(parent) => {
            // 两个 thinking delta 收进同一个面板（TextDelta 在 body 层级，
            // 不影响父面板内的聚合）
            assert_eq!(parent.elements.len(), 1);
            let CardElement::CollapsiblePanel(panel) = &parent.elements[0] else {
                panic!("expected thinking CollapsiblePanel");
            };
            match &panel.elements[0] {
                CardElement::Markdown { content } => assert_eq!(content, "first\nsecond"),
                other => panic!("expected Markdown, got {other:?}"),
            }
        }
        other => panic!("expected parent CollapsiblePanel, got {other:?}"),
    }
    match &body[1] {
        CardElement::Markdown { content } => assert_eq!(content, "interlude"),
        other => panic!("expected Markdown, got {other:?}"),
    }
}

#[test]
fn thinking_show_panel_header_is_thinking_label() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta {
            session_id: "s".into(),
            delta: "x".into(),
        },
        &cfg_show(),
    );
    let CardElement::CollapsiblePanel(parent) = &body[0] else {
        panic!("not a parent panel");
    };
    let CardElement::CollapsiblePanel(panel) = &parent.elements[0] else {
        panic!("not a thinking panel");
    };
    assert!(panel.header.title.content.contains("💭"));
    assert!(!panel.expanded, "默认折叠");
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
    // 默认折叠：body[0] 是父级面板，其内子面板标题 📖 Bash，args 在面板内。
    assert_eq!(body.len(), 1);
    match &body[0] {
        CardElement::CollapsiblePanel(parent) => {
            assert!(!parent.expanded, "父面板默认折叠");
            assert_eq!(parent.header.title.content, "🤔 折腾中");
            assert_eq!(parent.elements.len(), 1);
            match &parent.elements[0] {
                CardElement::CollapsiblePanel(tool_panel) => {
                    assert!(!tool_panel.expanded, "工具面板默认折叠");
                    assert_eq!(tool_panel.header.title.content, "📖 Bash");
                    assert!(matches!(
                        tool_panel.elements[0],
                        CardElement::Markdown { .. }
                    ));
                }
                other => panic!("expected tool CollapsiblePanel, got {other:?}"),
            }
        }
        other => panic!("expected parent CollapsiblePanel, got {other:?}"),
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
    // ToolStart + ToolProgress + ToolEnd 全部收进同一个折叠面板，
    // 该面板再收进父级工具面板。
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
    // 一个工具 = 父级面板内一行子面板；标题随生命周期变成 ✓ Bash。
    assert_eq!(body.len(), 1);
    match &body[0] {
        CardElement::CollapsiblePanel(parent) => {
            assert_eq!(parent.header.title.content, "🤔 折腾中");
            assert_eq!(parent.elements.len(), 1);
            match &parent.elements[0] {
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
                        CardElement::Markdown { content } => {
                            assert_eq!(content.chars().count(), 20)
                        }
                        other => panic!("expected result Markdown, got {other:?}"),
                    }
                }
                other => panic!("expected tool CollapsiblePanel, got {other:?}"),
            }
        }
        other => panic!("expected parent CollapsiblePanel, got {other:?}"),
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
        CardElement::CollapsiblePanel(parent) => {
            assert_eq!(parent.header.title.content, "🤔 折腾中");
            assert_eq!(parent.elements.len(), 1);
            match &parent.elements[0] {
                CardElement::CollapsiblePanel(panel) => {
                    assert_eq!(panel.header.title.content, "✓ Bash");
                    assert_eq!(panel.elements.len(), 1, "只有 args，没有结果内容");
                }
                other => panic!("expected tool CollapsiblePanel, got {other:?}"),
            }
        }
        other => panic!("expected parent CollapsiblePanel, got {other:?}"),
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
        CardElement::CollapsiblePanel(parent) => {
            assert_eq!(parent.header.title.content, "🤔 折腾中");
            assert_eq!(parent.elements.len(), 1);
            match &parent.elements[0] {
                CardElement::CollapsiblePanel(panel) => {
                    // [args markdown, 硬上限截断后的结果 markdown, 截断灰注]
                    assert_eq!(panel.elements.len(), 3);
                    match &panel.elements[1] {
                        CardElement::Markdown { content } => {
                            assert_eq!(content.chars().count(), 10240)
                        }
                        other => panic!("expected Markdown, got {other:?}"),
                    }
                    match &panel.elements[2] {
                        CardElement::Div { text } => assert!(text.content.contains("已截断 10 字")),
                        other => panic!("expected truncation note, got {other:?}"),
                    }
                }
                other => panic!("expected tool CollapsiblePanel, got {other:?}"),
            }
        }
        other => panic!("expected parent CollapsiblePanel, got {other:?}"),
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
fn finished_is_noop() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::Finished {
            session_id: "s".into(),
        },
        &cfg(),
    );
    assert!(body.is_empty(), "Finished 不再向卡片追加元素");
}

#[test]
fn error_append_markdown() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::Error {
            session_id: "s".into(),
            message: "boom".into(),
            terminal: false,
        },
        &cfg(),
    );
    assert_eq!(body.len(), 1);
    match &body[0] {
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

#[test]
fn parent_element_count_limit_drops_oldest_child() {
    // 父面板子元素超过 PARENT_ELEMENT_LIMIT(80) 时，最旧的子元素被丢弃。
    let mut body = vec![];
    // 先创建父面板 + 第一个工具面板
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"cmd":"echo 1"}),
        },
        &cfg(),
    );
    // 再添加 81 个工具，总工具数 = 82，超过 80 上限
    for i in 2..=82 {
        apply_event_to_card(
            &mut body,
            &AcpEvent::ToolStart {
                session_id: "s".into(),
                tool_name: format!("Tool{i}"),
                args: serde_json::json!({"cmd": format!("echo {i}")}),
            },
            &cfg(),
        );
    }
    // 父面板应在 80 个递归元素限制内（每个工具面板自身 + 内部 Markdown 计 2 个元素）
    match &body[0] {
        CardElement::CollapsiblePanel(parent) => {
            assert_eq!(parent.header.title.content, "🤔 折腾中");
            // 82 工具面板 × 2 + 1(父面板) = 165 > 80 → 丢到 39 个工具面板 = 79 个元素
            let remaining = parent.elements.len();
            assert!(remaining > 0, "should have some tool panels left");
            assert!(
                remaining <= 39,
                "should have at most 39 tool panels (79 elements)"
            );
            // 第一个子面板应是 Tool44（Bash + Tool2~Tool43 被丢弃了）
            let CardElement::CollapsiblePanel(first) = &parent.elements[0] else {
                panic!("expected tool panel");
            };
            assert_eq!(first.header.title.content, "📖 Tool44");
            // 最后一个子面板应是 Tool82
            let CardElement::CollapsiblePanel(last) = &parent.elements[remaining - 1] else {
                panic!("expected tool panel");
            };
            assert_eq!(last.header.title.content, "📖 Tool82");
        }
        other => panic!("expected parent CollapsiblePanel, got {other:?}"),
    }
}

#[test]
fn progress_note_limit_keeps_only_latest() {
    // 工具面板内最多保留 MAX_PROGRESS_NOTES(5) 条进度通知，超过时丢弃最旧的。
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart {
            session_id: "s".into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"cmd":"sleep"}),
        },
        &cfg_fold_tool(),
    );
    // 发送 8 条进度通知，应该只保留最后 5 条
    for i in 1..=8 {
        apply_event_to_card(
            &mut body,
            &AcpEvent::ToolProgress {
                session_id: "s".into(),
                tool_name: "Bash".into(),
                progress: format!("step {i}"),
            },
            &cfg_fold_tool(),
        );
    }
    // 验证进度通知数量
    match &body[0] {
        CardElement::CollapsiblePanel(parent) => {
            let CardElement::CollapsiblePanel(panel) = &parent.elements[0] else {
                panic!("expected tool panel");
            };
            let progress_notes: Vec<&CardElement> = panel
                .elements
                .iter()
                .filter(
                    |el| matches!(el, CardElement::Div { text } if text.content.starts_with("⏳ ")),
                )
                .collect();
            assert_eq!(progress_notes.len(), 5, "最多保留 5 条进度通知");
            // 最后一条应是 "step 8"
            let last_note = progress_notes.last().unwrap();
            let last_content = match last_note {
                CardElement::Div { text } => &text.content,
                _ => panic!("expected Div"),
            };
            assert!(last_content.contains("step 8"));
            // 第一条应是 "step 4"（step 1-3 被丢弃）
            let first_note = progress_notes.first().unwrap();
            let first_content = match first_note {
                CardElement::Div { text } => &text.content,
                _ => panic!("expected Div"),
            };
            assert!(first_content.contains("step 4"));
        }
        other => panic!("expected parent CollapsiblePanel, got {other:?}"),
    }
}
