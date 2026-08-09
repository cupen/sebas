use feishu::cards::{render_permission_card, render_root_card};

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
    assert_eq!(c.max_tool_output_chars, 1024);
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

#[test]
fn render_accumulated_card_structure() {
    use feishu::cards::{CardElement, render_accumulated_card};
    let body = vec![
        CardElement::Markdown {
            content: "hello".into(),
        },
        CardElement::Hr,
        CardElement::Markdown {
            content: "world".into(),
        },
    ];
    let card = render_accumulated_card("重构 foo", "msg_9", "🚧", &body, "orange");
    let s = serde_json::to_string(&card).unwrap();
    // header title 含 emoji + "Claude Code"，template=orange
    assert!(s.contains("🚧 Claude Code"));
    assert!(s.contains("\"template\":\"orange\""));
    // 引用块
    assert!(s.contains("> 重构 foo"));
    // body 两段 text 都在
    assert!(s.contains("hello"));
    assert!(s.contains("world"));
    // footer msg_id
    assert!(s.contains("msg_id: msg_9"));
}

#[test]
fn render_accumulated_card_empty_body_matches_seed() {
    use feishu::cards::render_accumulated_card;
    let card = render_accumulated_card("hi", "msg_1", "👀", &[], "blue");
    let s = serde_json::to_string(&card).unwrap();
    assert!(s.contains("👀 Claude Code"));
    assert!(s.contains("> hi"));
    assert!(s.contains("msg_id: msg_1"));
}

#[test]
fn permission_card_buttons_are_first_class_v2_elements() {
    use feishu::cards::CardElement;
    let card = render_permission_card("s1", "r1", "Bash", &serde_json::json!({"cmd": "ls"}));
    // Card JSON 2.0 removed the V1 `action` container (API error 200861
    // "unsupported tag action"), so the 3 decision buttons are first-class
    // body elements (stacked vertically, full width).
    let button_count = card
        .body
        .elements
        .iter()
        .filter(|e| matches!(e, CardElement::Button { .. }))
        .count();
    assert_eq!(button_count, 3, "3 first-class V2 button elements");

    // Wire format: no action container anywhere; each button carries a
    // `behaviors: [{type: "callback", value: {...}}]` wrapper, otherwise
    // Feishu silently ignores the button (no click callback registered).
    let s = serde_json::to_string(&card).unwrap();
    assert!(
        !s.contains("\"tag\":\"action\""),
        "V2 forbids the action container"
    );
    assert!(
        !s.contains("\"actions\":["),
        "no actions array without a container"
    );
    assert!(
        s.contains("\"tag\":\"button\""),
        "each button needs explicit tag:button"
    );
    assert!(
        s.contains("\"behaviors\":["),
        "each button must have behaviors array"
    );
    assert!(
        s.contains("\"type\":\"callback\""),
        "behavior type must be callback"
    );
    // 3 decisions.
    assert!(s.contains("本次允许"));
    assert!(s.contains("本会话不再询问"));
    assert!(s.contains("拒绝"));
}

#[test]
fn permission_card_args_in_code_fence_and_explanation_note() {
    let card = render_permission_card("s1", "r1", "Bash", &serde_json::json!({"cmd": "ls /tmp"}));
    let s = serde_json::to_string(&card).unwrap();
    // Args rendered in a JSON code fence, not a plain grey note.
    // Inside the JSON-of-the-card the fence's quotes are escaped as \", so
    // look for the escaped forms.
    assert!(s.contains("```json"), "args must be in a fenced code block");
    assert!(
        s.contains("\\\"cmd\\\""),
        "args must contain escaped cmd key"
    );
    assert!(s.contains("ls /tmp"), "args must contain the command value");
    // Explanation note present so users know what 本会话不再询问 means.
    assert!(s.contains("本会话不再询问 = 之后本会话所有权限请求自动放行"));
    assert!(s.contains("/new 或会话结束后失效"));
}

#[test]
fn collapsible_panel_serializes_v2_shape() {
    use feishu::cards::{
        CardElement, CardText, CollapsiblePanel, CollapsiblePanelHeader, StandardIcon,
    };
    let panel = CardElement::CollapsiblePanel(CollapsiblePanel {
        expanded: false,
        header: CollapsiblePanelHeader {
            title: CardText {
                tag: "plain_text".into(),
                content: "Bash 输出".into(),
            },
            icon: StandardIcon {
                tag: "standard_icon".into(),
                token: "down-small-ccm_outlined".into(),
                size: "16px 16px".into(),
            },
            icon_position: "right".into(),
            icon_expanded_angle: -180,
        },
        elements: vec![CardElement::Markdown {
            content: "long output".into(),
        }],
    });
    let s = serde_json::to_string(&panel).unwrap();
    assert!(s.contains("\"tag\":\"collapsible_panel\""));
    assert!(s.contains("\"expanded\":false"));
    assert!(s.contains("\"header\":{"));
    assert!(s.contains("\"elements\":["));
    assert!(s.contains("long output"));
}
