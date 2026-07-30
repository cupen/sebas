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
