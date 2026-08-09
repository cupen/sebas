use feishu::cards::{render_permission_card, render_root_card};

#[test]
fn card_config_thinking_default_is_show() {
    let cfg = feishu::cards::CardConfig::default();
    assert_eq!(cfg.thinking, feishu::cards::ThinkingDisplay::Show);
}

#[test]
fn card_config_serializes_thinking_as_lowercase() {
    let cfg = feishu::cards::CardConfig::default();
    let v = serde_json::to_value(&cfg).unwrap();
    assert_eq!(v["thinking"], "show");
}

#[test]
fn card_config_deserializes_thinking_from_lowercase() {
    let v = serde_json::json!({ "thinking": "hide" });
    let cfg: feishu::cards::CardConfig = serde_json::from_value(v).unwrap();
    assert_eq!(cfg.thinking, feishu::cards::ThinkingDisplay::Hide);
}

#[test]
fn card_config_rejects_unknown_thinking_value() {
    let v = serde_json::json!({ "thinking": "disable" });
    let res: Result<feishu::cards::CardConfig, _> = serde_json::from_value(v);
    assert!(res.is_err(), "disable is not exposed yet, must fail to parse");
}

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
    assert_eq!(c.max_tool_output_chars, 0);
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
fn permission_card_bash_args_rendered_as_command_headline() {
    let card = render_permission_card("s1", "r1", "Bash", &serde_json::json!({"cmd": "ls /tmp"}));
    let s = serde_json::to_string(&card).unwrap();
    // 命令独立成行内代码摘要，不再整段 JSON。
    assert!(
        s.contains("`$ ls /tmp`"),
        "command must be an inline-code headline: {s}"
    );
    assert!(
        !s.contains("```json"),
        "no raw JSON wall for flat Bash args: {s}"
    );
    // 说明 note 保留。
    assert!(s.contains("本会话不再询问 = 之后本会话所有权限请求自动放行"));
    assert!(s.contains("/new 或会话结束后失效"));
}

#[test]
fn permission_card_bash_extra_args_use_field_rows() {
    let card = render_permission_card(
        "s1",
        "r1",
        "Bash",
        &serde_json::json!({"command": "ls -la /tmp", "timeout": 30, "description": "list tmp"}),
    );
    let s = serde_json::to_string(&card).unwrap();
    assert!(s.contains("`$ ls -la /tmp`"));
    // 其余参数走 div.fields 的加粗 label + value 行。
    assert!(s.contains("\"fields\":["));
    assert!(s.contains("**超时**"));
    assert!(s.contains("**描述**"));
    assert!(s.contains("30"));
    assert!(s.contains("list tmp"));
}

#[test]
fn permission_card_file_path_headline_and_fields() {
    let card = render_permission_card(
        "s1",
        "r1",
        "Read",
        &serde_json::json!({"file_path": "src/main.rs", "offset": 100, "limit": 200}),
    );
    let s = serde_json::to_string(&card).unwrap();
    assert!(s.contains("📄 `src/main.rs`"));
    assert!(s.contains("**起始行**"));
    assert!(s.contains("**读取行数**"));
    assert!(!s.contains("```json"), "flat args must not fall back to JSON");
}

#[test]
fn permission_card_long_content_preview_and_panel() {
    let long = "x".repeat(500);
    let card = render_permission_card(
        "s1",
        "r1",
        "Write",
        &serde_json::json!({"file_path": "src/out.txt", "content": long}),
    );
    let s = serde_json::to_string(&card).unwrap();
    // 行内预览截断到 300 字符，完整 500 字符收进折叠面板。
    assert!(s.contains("…"), "preview must be truncated with ellipsis");
    assert!(s.contains("\"tag\":\"collapsible_panel\""));
    assert!(s.contains("完整参数"));
    // 完整内容只在折叠面板出现一次：行内预览已截断到 300，不会出现第二段 500。
    assert_eq!(
        s.matches(&"x".repeat(500)).count(),
        1,
        "full content must live in the panel only"
    );
}

#[test]
fn permission_card_nested_args_fall_back_to_json_fence() {
    let card = render_permission_card(
        "s1",
        "r1",
        "MCP::custom",
        &serde_json::json!({"items": [{"a": 1}, {"b": 2}], "nested": {"deep": true}}),
    );
    let s = serde_json::to_string(&card).unwrap();
    assert!(s.contains("```json"), "nested args keep the JSON fence");
    assert!(s.contains("\\\"items\\\""));
}

#[test]
fn permission_card_hard_limit_caps_giant_args() {
    let huge = "z".repeat(20_000);
    let card = render_permission_card(
        "s1",
        "r1",
        "Write",
        &serde_json::json!({"file_path": "f", "content": huge}),
    );
    let s = serde_json::to_string(&card).unwrap();
    // 完整参数面板受 8192 硬上限截断，行内预览 300。
    assert!(s.contains("参数过长，已截断"));
    assert!(
        !s.contains(&"z".repeat(8193)),
        "panel must cap the giant arg below 8193 chars"
    );
    assert!(
        s.contains(&"z".repeat(8000)),
        "capped panel keeps an 8000-char prefix"
    );
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
