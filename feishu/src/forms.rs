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
//! 实体。通用 CRUD 状态机在 `router::crud`（存储 trait + 列表/增删改流程）。

use crate::cards::{Card, CardBehavior, CardElement, CardText};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// 单选下拉的一个选项。
#[derive(Debug, Clone)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

/// 单个表单字段的最小集合：文本输入 + 单选下拉。
#[derive(Debug, Clone)]
pub enum FormField {
    Text {
        name: String,
        label: String,
        required: bool,
        placeholder: String,
        /// 敏感字段（如 API key）：列表展示掩码、编辑不预填、
        /// 提交留空时保留旧值（见 `router::crud`）。
        secret: bool,
    },
    Select {
        name: String,
        label: String,
        required: bool,
        options: Vec<SelectOption>,
    },
}

impl FormField {
    pub fn name(&self) -> &str {
        match self {
            FormField::Text { name, .. } | FormField::Select { name, .. } => name,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            FormField::Text { label, .. } | FormField::Select { label, .. } => label,
        }
    }

    pub fn required(&self) -> bool {
        match self {
            FormField::Text { required, .. } | FormField::Select { required, .. } => *required,
        }
    }

    /// 是否为敏感字段（掩码/不预填/留空保留）。
    pub fn is_secret(&self) -> bool {
        match self {
            FormField::Text { secret, .. } => *secret,
            FormField::Select { .. } => false,
        }
    }
}

/// 一张表单卡片的完整描述。同一个表单的列表/编辑/提交回调都通过
/// `form_name` 路由（回调负载里的 `form` 字段）。
#[derive(Debug, Clone)]
pub struct FormSpec {
    /// 表单容器唯一标识（`form.name`），同时是回调路由的 key。
    pub form_name: String,
    /// 卡片标题。
    pub title: String,
    /// 卡片主题色（header template，如 "blue" / "orange"）。
    pub template: String,
    pub fields: Vec<FormField>,
    pub submit_label: String,
}

impl FormSpec {
    pub fn new(
        form_name: impl Into<String>,
        title: impl Into<String>,
        fields: Vec<FormField>,
    ) -> Self {
        Self {
            form_name: form_name.into(),
            title: title.into(),
            template: "blue".into(),
            fields,
            submit_label: "提交".into(),
        }
    }
}

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
        match f {
            FormField::Text {
                name,
                required,
                placeholder,
                ..
            } => {
                elements.push(to_value(FormInput {
                    tag: "input",
                    placeholder: CardText {
                        tag: "plain_text".into(),
                        content: placeholder.clone(),
                    },
                    width: "fill",
                    name: name.clone(),
                    required: *required,
                    default_value: initial.get(name).cloned(),
                }));
            }
            FormField::Select {
                name,
                required,
                options,
                ..
            } => {
                elements.push(to_value(FormSelect {
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
                    required: *required,
                    name: name.clone(),
                }));
            }
        }
    }

    // 提交按钮：`form_action_type: "submit"` 触发整批表单数据回调。
    // 注意：容器内的 reset 按钮不允许带 behaviors 回调（API 11310），
    // 所以「取消/返回列表」由调用方放在表单容器外渲染（见 router::crud）。
    elements.push(to_value(FormButton {
        tag: "button",
        text: CardText {
            tag: "plain_text".into(),
            content: spec.submit_label.clone(),
        },
        r#type: "primary",
        behaviors: vec![CardBehavior {
            r#type: "callback".into(),
            value: submit,
        }],
        form_action_type: "submit",
        name: format!("{}_submit", spec.form_name),
    }));

    card.body.elements.push(CardElement::Form {
        name: spec.form_name.clone(),
        elements,
    });
    card
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

fn label_element(f: &FormField) -> Value {
    let star = if f.required() { " *" } else { "" };
    json!({ "tag": "markdown", "content": format!("**{}**{}", f.label(), star) })
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
}

#[derive(Serialize)]
struct FormSelectOption {
    text: CardText,
    value: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}
