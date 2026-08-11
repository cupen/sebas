use feishu::forms::{FormField, FormSpec, SelectOption, render_form_card, values_to_strings};
use std::collections::BTreeMap;

fn demo_spec() -> FormSpec {
    FormSpec::new(
        "note",
        "备忘",
        vec![
            FormField::Text {
                name: "title".into(),
                label: "标题".into(),
                required: true,
                placeholder: "标题".into(),
                secret: false,
                disabled: false,
            },
            FormField::Text {
                name: "body".into(),
                label: "内容".into(),
                required: false,
                placeholder: "正文".into(),
                secret: false,
                disabled: false,
            },
            FormField::Select {
                name: "priority".into(),
                label: "优先级".into(),
                required: true,
                options: vec![
                    SelectOption {
                        value: "p0".into(),
                        label: "P0".into(),
                    },
                    SelectOption {
                        value: "p1".into(),
                        label: "P1".into(),
                    },
                ],
                on_change: None,
            },
        ],
    )
}

#[test]
fn form_card_serializes_v2_form_container() {
    let mut initial = BTreeMap::new();
    initial.insert("title".into(), "旧标题".into());
    let submit = serde_json::json!({ "form": "note", "op": "submit", "id": "n1" });
    let card = render_form_card(&demo_spec(), &initial, submit);
    let s = serde_json::to_string(&card).unwrap();

    // 卡片根节点直接放 form 容器（JSON 2.0 约束：不能嵌套）。
    assert!(s.contains("\"schema\":\"2.0\""));
    assert!(s.contains("\"tag\":\"form\""));
    assert!(s.contains("\"name\":\"note\""));

    // 文本输入：必填、占位符、编辑预填 default_value。
    assert!(s.contains("\"tag\":\"input\""));
    assert!(s.contains("\"name\":\"title\""));
    assert!(s.contains("\"required\":true"));
    assert!(s.contains("\"default_value\":\"旧标题\""));
    // 非必填输入不输出 required:false。
    assert!(!s.contains("\"required\":false"));

    // 单选下拉：选项 value/label 齐全。
    assert!(s.contains("\"tag\":\"select_static\""));
    assert!(s.contains("\"value\":\"p0\""));
    assert!(s.contains("\"content\":\"P0\""));

    // 必填字段 label 带星号；提交按钮绑定 form_action_type。
    assert!(s.contains("**优先级** *"));
    assert!(s.contains("\"form_action_type\":\"submit\""));
    // 提交按钮的回传负载原样携带（op/id）。
    assert!(s.contains("\"op\":\"submit\""));
    assert!(s.contains("\"id\":\"n1\""));
    // 容器内不允许带 behaviors 的 reset 按钮（API 11310），渲染层不输出。
    assert!(!s.contains("\"form_action_type\":\"reset\""));
}

#[test]
fn form_card_snapshot() {
    let mut initial = BTreeMap::new();
    initial.insert("title".into(), "旧标题".into());
    let card = render_form_card(
        &demo_spec(),
        &initial,
        serde_json::json!({ "form": "note", "op": "submit", "id": "n1" }),
    );
    insta::assert_yaml_snapshot!(card);
}

#[test]
fn values_to_strings_flattens_scalars() {
    let mut fv = BTreeMap::new();
    fv.insert("title".into(), serde_json::json!("hi"));
    fv.insert("priority".into(), serde_json::json!("p0"));
    fv.insert("count".into(), serde_json::json!(3));
    fv.insert("flag".into(), serde_json::json!(true));
    let out = values_to_strings(&fv);
    assert_eq!(out.get("title").map(String::as_str), Some("hi"));
    assert_eq!(out.get("priority").map(String::as_str), Some("p0"));
    assert_eq!(out.get("count").map(String::as_str), Some("3"));
    assert_eq!(out.get("flag").map(String::as_str), Some("true"));
}

/// 重新渲染时 single-select 必须带上 `initial_value`，否则下拉只能展示
/// placeholder，看起来像 on_change 没生效（provider preset → base_url 自动
/// 填充的视觉断点就是这个）。空字符串与 None 都不输出 `initial_value`。
#[test]
fn select_renders_initial_value_for_preselected_option() {
    let mut initial = BTreeMap::new();
    initial.insert("priority".into(), "p0".into());
    let card = render_form_card(
        &demo_spec(),
        &initial,
        serde_json::json!({ "form": "note", "op": "submit" }),
    );
    let s = serde_json::to_string(&card).unwrap();
    assert!(
        s.contains("\"initial_value\":[\"p0\"]"),
        "select 缺 initial_value，重渲时无法高亮已选项：{s}"
    );

    // 没填/空字符串 → 不输出 initial_value（保留原 placeholder 行为）。
    let empty = render_form_card(
        &demo_spec(),
        &BTreeMap::new(),
        serde_json::json!({ "form": "note", "op": "submit" }),
    );
    let s = serde_json::to_string(&empty).unwrap();
    assert!(
        !s.contains("initial_value"),
        "无预选时不应输出 initial_value：{s}"
    );

    let blank = render_form_card(
        &demo_spec(),
        &BTreeMap::from([("priority".into(), String::new())]),
        serde_json::json!({ "form": "note", "op": "submit" }),
    );
    let s = serde_json::to_string(&blank).unwrap();
    assert!(
        !s.contains("initial_value"),
        "空字符串等同于无预选：{s}"
    );
}
