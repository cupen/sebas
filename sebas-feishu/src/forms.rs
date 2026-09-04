//! 可复用的卡片表单原语（card JSON 2.0 `form` 容器）。
//!
//! 官方行为：表单容器允许用户在前端本地录入一批表单项，点击一次「提交」后，
//! 整批数据以 `action.form_value`（组件 `name` -> 值）一次回调到服务端。
//! 关键约束（实现时已遵守）：
//! - 表单容器只能放在卡片根节点，不能嵌套其它容器；
//! - 容器内所有交互组件必须有全局唯一的 `name`，否则提交回调失败（200530）；
//! - 容器内必须至少有一个 `form_action_type: "submit"` 的按钮；
//! - 表单容器客户端要求 V6.6+，老版本显示 fallback。
//!
//! 本模块只负责「schema → 表单卡片」的渲染与回调值整理，不绑定任何业务
//! 实体。通用 CRUD 状态机在 `sebas_router::crud`（存储 trait + 列表/增删改流程）。
//!
//! 类型边界的说明（decouple-feishu-channel task 3）：schema 类型（`FormSpec`
//! / `FormField` / `SelectOption`）已经**中立化**到 `sebas-channels`，router 与
//! webui 消费中立类型；这里 re-export 保持既有 import 路径
//! （`sebas_feishu::forms::{FormField, FormSpec, SelectOption, ...}`）不变。
//! 剩下本模块专属的**渲染**原语（`render_form_card`、`values_to_strings`、
//! `input_element` 等）负责把 schema 渲染成飞书 form 容器 wire 元素。

pub use sebas_channels::card::{FormField, FormSpec, SelectOption};

use crate::cards::{Card, CardBehavior, CardElement, CardText};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// 渲染一张表单卡片。`initial` 提供编辑场景的预填值（按字段 `name`）；
/// `submit` 是提交按钮 `behaviors[].value` 的自定义回传负载（如
/// `{"form": ..., "op": "submit", "id": ...}`），Feishu 会在提交回调的
/// `action.value` 里原样返回。
pub fn render_form_card(
    spec: &FormSpec,
    initial: &BTreeMap<String, String>,
    submit: Value,
) -> Card {
    let mut card = Card::new(&spec.title, &spec.template);
    let mut elements: Vec<Value> = Vec::new();

    for f in &spec.fields {
        elements.push(label_element(f));
        // 单选预填：value 非空时包成 Vec(len=1)，让重新渲染的卡片保持
        // 已选项可见（否则下拉只能展示 placeholder，看起来像 on_change 没生效）。
        let select_initial: Option<Vec<String>> = initial
            .get(f.name())
            .filter(|v| !v.is_empty())
            .map(|v| vec![v.clone()]);
        match f {
            FormField::Text {
                name,
                required,
                placeholder,
                disabled,
                ..
            } => {
                elements.push(input_element(
                    name,
                    placeholder,
                    *required,
                    initial.get(name).cloned(),
                    *disabled,
                ));
            }
            FormField::Select {
                name,
                required,
                options,
                on_change,
                ..
            } => {
                elements.push(select_element(
                    name,
                    options,
                    *required,
                    select_initial,
                    on_change.as_ref(),
                ));
            }
        }
    }

    // 提交按钮：`form_action_type: "submit"` 触发整批表单数据回调。
    // 注意：容器内的 reset 按钮不允许带 behaviors 回调（API 11310），
    // 所以「取消/返回列表」由调用方放在表单容器外渲染（见 sebas_router::crud）。
    elements.push(submit_button(&spec.form_name, &spec.submit_label, &submit));

    card.body.elements.push(CardElement::Form {
        name: spec.form_name.clone(),
        elements,
    });
    card
}

/// 表单容器内字段的 label 元素。
pub(crate) fn label_element(f: &FormField) -> Value {
    let star = if f.required() { " *" } else { "" };
    json!({ "tag": "markdown", "content": format!("**{}**{}", f.label(), star) })
}

/// 单个文本 `input` form 元素（飞书 form 容器 shape）。
pub(crate) fn input_element(
    name: &str,
    placeholder: &str,
    required: bool,
    initial: Option<String>,
    disabled: bool,
) -> Value {
    to_value(FormInput {
        tag: "input",
        placeholder: CardText {
            tag: "plain_text".into(),
            content: placeholder.to_string(),
        },
        width: "fill",
        name: name.to_string(),
        required,
        default_value: initial,
        disabled,
    })
}

/// 单个 `select_static` form 元素（飞书 form 容器 shape；容器内不接受
/// tag="select"）。交互回传挂 `behaviors`（on_change）。
pub(crate) fn select_element(
    name: &str,
    options: &[SelectOption],
    required: bool,
    initial: Option<Vec<String>>,
    on_change: Option<&Value>,
) -> Value {
    let behaviors = on_change.map(|v| {
        vec![CardBehavior {
            r#type: "callback".into(),
            value: v.clone(),
        }]
    });
    to_value(FormSelect {
        tag: "select_static",
        placeholder: CardText {
            tag: "plain_text".into(),
            content: "请选择".into(),
        },
        options: options
            .iter()
            .map(|o| FormSelectOption {
                text: CardText {
                    tag: "plain_text".into(),
                    content: o.label.clone(),
                },
                value: o.value.clone(),
            })
            .collect(),
        width: "fill",
        r#type: "default",
        required,
        name: name.to_string(),
        behaviors,
        initial_value: initial,
    })
}

/// 提交按钮元素（`form_action_type: "submit"`）。
pub(crate) fn submit_button(form_name: &str, label: &str, submit: &Value) -> Value {
    to_value(FormButton {
        tag: "button",
        text: CardText {
            tag: "plain_text".into(),
            content: label.to_string(),
        },
        r#type: "primary",
        behaviors: vec![CardBehavior {
            r#type: "callback".into(),
            value: submit.clone(),
        }],
        form_action_type: "submit",
        name: format!("{form_name}_submit"),
    })
}

/// 把 `action.form_value` 回调按组件 `name` 整理成字符串表。
/// 文本/单选取值是字符串；数字/布尔兜底转字符串，嵌套值保留 JSON 文本，
/// 供存储层使用（多选数组等更复杂字段后续按需扩展）。
pub fn values_to_strings(form_value: &BTreeMap<String, Value>) -> BTreeMap<String, String> {
    form_value
        .iter()
        .map(|(k, v)| (k.clone(), scalar_string(v)))
        .collect()
}

fn scalar_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn to_value(v: impl Serialize) -> Value {
    serde_json::to_value(v).expect("form element serializes")
}

#[derive(Serialize)]
struct FormButton {
    tag: &'static str,
    text: CardText,
    r#type: &'static str,
    behaviors: Vec<CardBehavior>,
    form_action_type: &'static str,
    name: String,
}

#[derive(Serialize)]
struct FormInput {
    tag: &'static str,
    placeholder: CardText,
    width: &'static str,
    name: String,
    #[serde(skip_serializing_if = "is_false")]
    required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_value: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    disabled: bool,
}

#[derive(Serialize)]
struct FormSelect {
    tag: &'static str,
    placeholder: CardText,
    options: Vec<FormSelectOption>,
    width: &'static str,
    r#type: &'static str,
    #[serde(skip_serializing_if = "is_false")]
    required: bool,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    behaviors: Option<Vec<CardBehavior>>,
    /// 预选中的 option value 列表（单选 len=1；None 表示无预选）。
    /// 飞书 select/select_static 都用同名 `initial_value` 字段。
    /// 没有这个字段的话，重新渲染时下拉只能展示 placeholder，看起来像
    /// `on_change` 没生效（典型：选 provider preset 后 base_url 填了但
    /// 下拉没高亮）。
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_value: Option<Vec<String>>,
}

#[derive(Serialize)]
struct FormSelectOption {
    text: CardText,
    value: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}
