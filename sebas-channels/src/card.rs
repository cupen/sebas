//! Neutral outbound presentation (design D4): the channel-agnostic card
//! model — per-turn title/theme plus a body of elements with the same
//! semantic vocabulary the feishu cards used (hr / markdown / div / button /
//! fields / collapsible panel / form / select / column set). Adapters render
//! this into their native presentation (feishu: card schema 2.0 JSON).
//! Interactive payloads (`Button.behaviors[].value`, `SelectStatic.on_change`,
//! `Form.submit`) are opaque `serde_json::Value` blobs defined by the router's
//! callback protocol and parsed back by the adapter.

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Neutral `Fields` needs a struct serializer: serde rejects internally-tagged
/// newtype variants whose payload is a sequence ("cannot serialize tagged
/// newtype variant ... containing a sequence"), so we emit the two fields
/// explicitly.
fn serialize_fields<S: serde::Serializer>(
    fields: &[Field],
    ser: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeStruct;
    let mut s = ser.serialize_struct("ChannelElement", 2)?;
    s.serialize_field("tag", "fields")?;
    s.serialize_field("fields", fields)?;
    s.end()
}

/// Turn chrome for the accumulated presentation: the per-turn framing data
/// (user prompt + turn-context footer) the adapter needs to build its native
/// card frame. `None` when the presentation carries no turn framing (discrete
/// UI cards — help, provider, permission, error/status).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TurnChrome {
    /// The user prompt that started this turn (the adapter renders it as the
    /// frame's quote block and derives the header topic from its first line).
    pub prompt: String,
    /// Turn/session identifier for the `msg_id: {session_id}` fallback footer.
    pub session_id: String,
    /// Cumulative usage (`AppUsage`) when the router has seen a usage event.
    /// When `Some`, the adapter renders the model/token footer instead of the
    /// `msg_id:` footer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<AppUsage>,
}

/// The neutral equivalent of the feishu usage footer data (`CardFooter`):
/// cumulative totals only (the round counters are never rendered — the footer
/// contract is `{model} · in: {total_input} out: {total_output} ·
/// ctx: {total_input}`).
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct AppUsage {
    /// Full model name (e.g. `claude-sonnet-4-20250514`); the adapter shortens
    /// it for display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub total_input: u64,
    pub total_output: u64,
}

/// One outbound presentation instance for a turn (was feishu `Card`):
/// title + theme colour + accumulated body elements + optional turn chrome.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChannelCard {
    pub title: String,
    /// Header theme colour (feishu header template: "blue", "orange", ...).
    pub theme: String,
    pub elements: Vec<ChannelElement>,
    /// Turn framing (prompt / session_id / usage). `None` for discrete UI
    /// cards (help, permission, provider, error/status).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnChrome>,
}

impl ChannelCard {
    pub fn new(title: impl Into<String>, theme: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            theme: theme.into(),
            elements: Vec::new(),
            turn: None,
        }
    }

    /// JSON text of the neutral card (the wire shape adapters translate).
    /// Convenience for tests/debugging: `{title, theme, elements, ...}`.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("neutral card serializes")
    }

    /// Push a markdown body element.
    pub fn push_text(&mut self, content: impl Into<String>) {
        self.elements.push(ChannelElement::Markdown {
            content: content.into(),
        });
    }

    /// Push a note-style grey `div` (notation size).
    pub fn push_note(&mut self, content: impl Into<String>) {
        self.elements.push(ChannelElement::Div {
            text: DivText {
                tag: "plain_text".into(),
                content: content.into(),
                text_size: Some("notation".into()),
                text_color: Some("grey".into()),
            },
        });
    }

    /// Push a divider (`hr`).
    pub fn push_divider(&mut self) {
        self.elements.push(ChannelElement::Hr);
    }

    /// Push one first-class v2 button; the click payload rides in
    /// `behaviors[].value`.
    pub fn push_button(&mut self, text: impl Into<String>, style: &str, value: Value) {
        self.elements.push(ChannelElement::Button {
            text: RichText {
                tag: "plain_text".into(),
                content: text.into(),
            },
            style: style.into(),
            behaviors: vec![Behavior {
                r#type: "callback".into(),
                value,
            }],
        });
    }

    /// Push several buttons (see [`ChannelCard::push_button`]).
    pub fn push_actions(&mut self, buttons: Vec<ButtonSpec>) {
        for b in buttons {
            self.push_button(b.text, &b.style, b.value);
        }
    }
}

/// One button's declaration for [`ChannelCard::push_actions`].
#[derive(Debug, Clone)]
pub struct ButtonSpec {
    pub text: String,
    pub style: String,
    pub value: Value,
}

impl ButtonSpec {
    pub fn new(text: impl Into<String>, style: &str, value: Value) -> Self {
        Self {
            text: text.into(),
            style: style.into(),
            value,
        }
    }
}

/// Body element vocabulary (mirrors the historical feishu card elements 1:1;
/// the adapter maps each variant to its native JSON counterpart). Each
/// variant serializes with a `tag` discriminant (snake_case) plus its
/// fields.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "tag", rename_all = "snake_case")]
pub enum ChannelElement {
    Hr,
    #[serde(rename = "markdown")]
    Markdown {
        #[serde(rename = "content")]
        content: String,
    },
    /// Note-style plain text (grey, small) — v2 `div` with text.
    #[serde(rename = "div")]
    Div {
        #[serde(rename = "text")]
        text: DivText,
    },
    /// First-class v2 button; callback payloads travel in `behaviors`.
    #[serde(rename = "button")]
    Button {
        #[serde(rename = "text")]
        text: RichText,
        /// "primary" | "danger" | "default"
        #[serde(rename = "type")]
        style: String,
        #[serde(rename = "behaviors")]
        behaviors: Vec<Behavior>,
    },
    /// Key-value field rows (`div.fields`).
    #[serde(serialize_with = "serialize_fields", rename = "fields")]
    Fields(Vec<Field>),
    /// Secondary/long content behind a tappable header, default collapsed.
    #[serde(rename = "collapsible_panel")]
    CollapsiblePanel(CollapsiblePanel),
    /// Form container: collects fields and submits in ONE callback.
    /// `submit` is the submit button's callback payload; `initials` prefills
    /// field values (edit views).
    #[serde(rename = "form")]
    Form {
        #[serde(rename = "name")]
        name: String,
        #[serde(rename = "fields")]
        fields: Vec<FormField>,
        #[serde(rename = "initials")]
        initials: BTreeMap<String, String>,
        #[serde(rename = "submit")]
        submit: Value,
    },
    /// Top-level dropdown; changing the selection fires a callback with
    /// `on_change` as payload and the chosen value keyed by `name`.
    #[serde(rename = "select_static")]
    SelectStatic {
        #[serde(rename = "name")]
        name: String,
        #[serde(rename = "placeholder")]
        placeholder: RichText,
        #[serde(rename = "options")]
        options: Vec<(String, String)>,
        #[serde(rename = "initial")]
        initial: Option<String>,
        #[serde(rename = "on_change")]
        on_change: Value,
    },
    /// Horizontal layout container.
    #[serde(rename = "column_set")]
    ColumnSet {
        #[serde(rename = "flex_mode")]
        flex_mode: bool,
        #[serde(rename = "horizontal_spacing")]
        horizontal_spacing: Option<String>,
        #[serde(rename = "columns")]
        columns: Vec<Column>,
    },
}

/// Text with a feishu tag (`plain_text` / `lark_md`); kept lossless so the
/// adapter's JSON mapping is mechanical.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RichText {
    pub tag: String,
    pub content: String,
}

impl RichText {
    pub fn plain(content: impl Into<String>) -> Self {
        Self {
            tag: "plain_text".into(),
            content: content.into(),
        }
    }
}

/// Note-style div text (notation size + grey colour).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DivText {
    pub tag: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_color: Option<String>,
}

/// One key-value field row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Field {
    pub is_short: bool,
    pub text: RichText,
}

/// A button's callback behaviour declaration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Behavior {
    pub r#type: String,
    pub value: Value,
}

/// Collapsible panel; `icon_token` selects the header icon (the adapter
/// fills the fixed icon boilerplate).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CollapsiblePanel {
    pub expanded: bool,
    pub header_title: RichText,
    pub icon_token: String,
    pub elements: Vec<ChannelElement>,
}

/// One column of a [`ChannelElement::ColumnSet`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Column {
    pub width: Option<String>,
    pub elements: Vec<ChannelElement>,
    pub vertical_spacing: Option<String>,
    pub horizontal_align: Option<String>,
}

/// Single select option inside a form.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

/// One form field: text input or single select (the minimal form vocabulary).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FormField {
    Text {
        name: String,
        label: String,
        required: bool,
        placeholder: String,
        /// Sensitive field (e.g. API key): masked in lists, not prefilled in
        /// edits, empty submit keeps the old value.
        secret: bool,
        /// Read-only display: value still submitted with the form.
        disabled: bool,
    },
    Select {
        name: String,
        label: String,
        required: bool,
        options: Vec<SelectOption>,
        /// 设了 `Some(payload)` 就把下拉渲染成交互式 `select_static`
        /// （选中即触发整张表单的当前值回调，服务端据此重渲）；
        /// `None` 时仍是静默下拉，只在表单提交时随整批数据上送。
        on_change: Option<Value>,
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

    /// Whether the field is required (renders a `*` beside the label).
    pub fn required(&self) -> bool {
        match self {
            FormField::Text { required, .. } | FormField::Select { required, .. } => *required,
        }
    }

    /// Text field's secret flag (masked display / no edit prefill / empty
    /// submit keeps the old value). Selects are never secret.
    pub fn is_secret(&self) -> bool {
        match self {
            FormField::Text { secret, .. } => *secret,
            FormField::Select { .. } => false,
        }
    }
}

/// A complete form description: form name (callback routing key) + display
/// title/theme + fields. The neutral twin of `sebas_feishu::forms::FormSpec`
/// (the feishu adapter re-uses this type for its own form rendering).
#[derive(Debug, Clone, PartialEq)]
pub struct FormSpec {
    /// 表单容器唯一标识（`form.name`），同时是回调路由的 key。
    pub form_name: String,
    /// 卡片标题。
    pub title: String,
    /// 卡片主题色（header template，如 "blue" / "orange"）。
    pub template: String,
    pub fields: Vec<FormField>,
    /// 提交按钮文案。
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

/// Coerce submitted form values (`form_value` map of JSON values) into the
/// string map the router's business logic consumes. Neutrally useful: the
/// values arrive over any channel's form callback.
pub fn values_to_strings(form_value: &BTreeMap<String, Value>) -> BTreeMap<String, String> {
    form_value
        .iter()
        .map(|(k, v)| {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            (k.clone(), s)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_card_roundtrip_through_serde() {
        let mut card = ChannelCard::new("编辑", "blue");
        card.elements.push(ChannelElement::Form {
            name: "provider-edit".into(),
            fields: vec![FormField::Text {
                name: "api_key".into(),
                label: "API Key".into(),
                required: false,
                placeholder: "sk-...".into(),
                secret: true,
                disabled: false,
            }],
            initials: BTreeMap::new(),
            submit: serde_json::json!({"session_id": "s1"}),
        });
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(json["title"], "编辑");
        assert_eq!(json["elements"][0]["tag"], "form");
        assert_eq!(json["elements"][0]["name"], "provider-edit");
        assert_eq!(json["elements"][0]["fields"][0]["name"], "api_key");
    }

    #[test]
    fn element_vocabulary_uses_tag_discriminant() {
        // `tag` mirrors the feishu card element vocabulary 1:1; the feishu
        // adapter's mechanical mapping keys off it.
        let mut card = ChannelCard::new("t", "blue");
        card.elements.push(ChannelElement::Hr);
        card.elements.push(ChannelElement::Button {
            text: RichText::plain("允许"),
            style: "primary".into(),
            behaviors: vec![Behavior {
                r#type: "callback".into(),
                value: serde_json::json!({"session_id": "s1"}),
            }],
        });
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(json["elements"][0]["tag"], "hr");
        assert_eq!(json["elements"][1]["tag"], "button");
        assert_eq!(json["elements"][1]["type"], "primary");
    }

    #[test]
    fn fields_newtype_serializes_as_tagged_object() {
        // Internally-tagged newtype variants carrying a Vec reject in serde
        // ("cannot serialize tagged newtype variant ... containing a
        // sequence"); `serialize_fields` emits the object explicitly.
        let mut card = ChannelCard::new("t", "blue");
        card.elements.push(ChannelElement::Fields(vec![Field {
            is_short: false,
            text: RichText::plain("label\nvalue"),
        }]));
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(json["elements"][0]["tag"], "fields");
        assert_eq!(json["elements"][0]["fields"][0]["is_short"], false);
    }

    #[test]
    fn turn_chrome_is_optional_and_skipped_when_absent() {
        let plain = ChannelCard::new("帮助", "blue");
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json.get("turn").is_none(), "no turn key when absent");

        let mut with_turn = ChannelCard::new("主题", "orange");
        with_turn.turn = Some(TurnChrome {
            prompt: "重构 foo".into(),
            session_id: "s1".into(),
            usage: Some(AppUsage {
                model: Some("claude-sonnet-4-20250514".into()),
                total_input: 5000,
                total_output: 3000,
            }),
        });
        let json = serde_json::to_value(&with_turn).unwrap();
        assert_eq!(json["turn"]["prompt"], "重构 foo");
        assert_eq!(json["turn"]["session_id"], "s1");
        assert_eq!(json["turn"]["usage"]["model"], "claude-sonnet-4-20250514");
        assert_eq!(json["turn"]["usage"]["total_input"], 5000);
    }

    #[test]
    fn values_to_strings_coerces_json_shapes() {
        let mut m = BTreeMap::new();
        m.insert("a".into(), Value::String("x".into()));
        m.insert("b".into(), serde_json::json!(3));
        m.insert("c".into(), Value::Null);
        let out = values_to_strings(&m);
        assert_eq!(out["a"], "x");
        assert_eq!(out["b"], "3");
        assert_eq!(out["c"], "");
    }
}
