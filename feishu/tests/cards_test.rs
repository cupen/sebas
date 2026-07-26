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
