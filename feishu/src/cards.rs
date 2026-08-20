use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Map a router FSM phase string (the Feishu `emoji_type`, e.g. `"Typing"`,
/// `"OnIt"`, `"DONE"`, `"CrossMark"`) to the Unicode glyph historically shown
/// in the card header. Feishu rejects arbitrary Unicode as reaction
/// `emoji_type` (error 231001), so the state stores the API name and we render
/// the visual separately. Unknown phases fall back to a neutral bullet so a bad
/// value can't break the card.
///
/// Card header titles are now topic-driven (`derive_topic`), so this mapping is
/// no longer wired into the rendered header — the FSM state itself is surfaced
/// as Feishu reactions (see `router::card_state::phase`). Kept as a pub helper
/// for tests and any downstream that still needs the glyph ↔ phase map.
// (derive_session_title 已废弃：UUID 短形式对用户不可读；标题回到
// `derive_topic(user_prompt)` 首行非空 prompt。)
pub fn phase_visual(phase: &str) -> &str {
    match phase {
        "Typing" => "👀",
        "OnIt" => "🚧",
        "DONE" => "✅",
        "CrossMark" => "❌",
        _ => "•",
    }
}

/// 卡片流配置（spec §7）。原 `[card]` TOML 段，解析后由 router/feishu 共用。
/// 落在 feishu crate（依赖链最底端），router 与 cards 均可引用。
///
/// `deny_unknown_fields`: 用户手写 `settings.json` 时写错字段名
/// （比如 `theme` 而非 `theme_color`）必须立刻报错，而不是被 serde 静默
/// 忽略后被下一次 save_settings 抹平。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CardConfig {
    #[serde(default = "default_theme_color")]
    pub theme_color: String,
    /// 单元素文本软上限：超过则截断 + 追加灰注（(已折叠 N 字)）。
    #[serde(default = "default_max_user_text")]
    pub max_user_text_chars: usize,
    /// tool result 软上限：0（默认）= 完全不输出 tool call 的结果内容；
    /// \>0 时结果收进工具折叠面板，超过该值则折叠，完整内容保留。
    /// 代码另有 10240 硬上限兜底，配置无法放宽。
    #[serde(default = "default_max_tool_output")]
    pub max_tool_output_chars: usize,
    /// true（默认）：tool call 折叠成 collapsible_panel（默认收起），
    /// tool result 按 max_tool_output_chars 屏蔽/收纳；
    /// false：不折叠，全文内联展示（结果仍受 10240 硬上限约束）。
    #[serde(default = "default_true")]
    pub fold_long_output: bool,
    #[serde(default)]
    pub thinking: ThinkingDisplay,
}

/// How to render the model's `thinking` content into the Feishu card.
/// `disable` is intentionally not exposed — reserved for a future
/// feature that would also turn off thinking tokens at the agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingDisplay {
    /// Fold each thinking burst into a collapsible_panel (default).
    #[default]
    Show,
    /// Drop ThinkingDelta events from the card body entirely. The model
    /// still produces thinking tokens; we just don't surface them.
    Hide,
}

impl Default for CardConfig {
    fn default() -> Self {
        Self {
            theme_color: default_theme_color(),
            max_user_text_chars: default_max_user_text(),
            max_tool_output_chars: default_max_tool_output(),
            fold_long_output: default_true(),
            thinking: ThinkingDisplay::default(),
        }
    }
}

fn default_theme_color() -> String {
    "blue".into()
}
fn default_max_user_text() -> usize {
    4000
}
fn default_max_tool_output() -> usize {
    0
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct Card {
    pub schema: String,
    pub header: CardHeader,
    pub body: CardBody,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardBody {
    pub elements: Vec<CardElement>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardHeader {
    pub title: CardTitle,
    pub template: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardTitle {
    pub content: String,
    pub tag: String,
}

#[derive(Debug, Clone)]
pub enum CardElement {
    Hr,
    Markdown {
        content: String,
    },
    /// V2 replacement for the removed V1 `note` component: plain text with
    /// notation size + grey color, per the card JSON 2.0 migration guide.
    Div {
        text: DivText,
    },
    /// V2 buttons are first-class body elements (the V1 `action` container
    /// was removed — schema-2.0 cards get error 200861 "unsupported tag
    /// action"); callback payloads travel via `behaviors`.
    Button {
        text: CardText,
        r#type: String,
        behaviors: Vec<CardBehavior>,
    },
    /// V2 `div` 的字段行组合：`fields` 数组里每个 field 是加粗 label + value
    /// （权限卡参数用 key-value 行展示，替代 JSON 代码墙）。
    Fields(Vec<CardField>),
    /// Card JSON 2.0 `collapsible_panel` container: secondary/long content
    /// behind a tappable header. Defaults to collapsed; Feishu renders it on
    /// client V7.9+ (older clients show an upgrade placeholder instead).
    CollapsiblePanel(CollapsiblePanel),
    /// Card JSON 2.0 `form` container: collects several form items locally
    /// and submits them in ONE callback (`action.form_value`) when the user
    /// clicks a `form_action_type: "submit"` button. Must sit at the card
    /// root (cannot be nested in other containers); inner interactive
    /// components need globally-unique `name`s. Inner elements are
    /// pre-serialized values produced by `crate::forms`.
    Form {
        name: String,
        elements: Vec<serde_json::Value>,
    },
    /// Card JSON 2.0 `select_static` rendered as a top-level body element
    /// (not inside a `Form` container). Used by the `/provider` card for
    /// 「Default provider for DIRECT」 and 「Provider list」 dropdowns.
    /// `behaviors` carries a callback payload so changing the selection
    /// fires `card.action.trigger` back to the router with the chosen
    /// value keyed by `name` in `action.form_value`.
    SelectStatic {
        name: String,
        placeholder: CardText,
        options: Vec<(String, String)>,
        /// Currently selected option's value; rendered with `initial_value`
        /// so the dropdown doesn't reset to placeholder after a refresh.
        initial: Option<String>,
        /// On-change callback payload (matches the format used by
        /// `CardElement::Button`'s `behaviors[].value`).
        on_change: Value,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct DivText {
    pub tag: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_color: Option<String>,
}

/// V2 `div.fields` 里的单个字段行：`is_short=false` 独占一行，label 加粗。
#[derive(Debug, Clone, Serialize)]
pub struct CardField {
    pub is_short: bool,
    pub text: CardText,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardBehavior {
    pub r#type: String,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardText {
    pub tag: String,
    pub content: String,
}

/// A callback button's declaration: `value` is the payload Feishu sends back
/// on click. Serialized as a first-class V2 body element via
/// [`CardElement::Button`] (see `push_actions`).
#[derive(Debug, Clone, Serialize)]
pub struct CardButton {
    pub text: CardText,
    pub r#type: String, // "primary" | "danger" | "default"
    pub value: Value,
}

/// V2 `collapsible_panel` container. `expanded=false` renders the panel
/// folded; tapping the header toggles it in the client.
#[derive(Debug, Clone, Serialize)]
pub struct CollapsiblePanel {
    pub expanded: bool,
    pub header: CollapsiblePanelHeader,
    pub elements: Vec<CardElement>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollapsiblePanelHeader {
    pub title: CardText,
    pub icon: StandardIcon,
    pub icon_position: String,
    pub icon_expanded_angle: i32,
}

/// Icon library reference (`standard_icon`) used in panel headers.
#[derive(Debug, Clone, Serialize)]
pub struct StandardIcon {
    pub tag: String,
    pub token: String,
    pub size: String,
}

impl Serialize for CardElement {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match self {
            CardElement::Hr => {
                // Feishu requires the tag on structural elements; without it
                // the card schema is invalid and the API rejects the card.
                let mut s = ser.serialize_struct("CardElement", 1)?;
                s.serialize_field("tag", "hr")?;
                s.end()
            }
            CardElement::Markdown { content } => {
                let mut s = ser.serialize_struct("CardElement", 2)?;
                s.serialize_field("tag", "markdown")?;
                s.serialize_field("content", content)?;
                s.end()
            }
            CardElement::Div { text } => {
                let mut s = ser.serialize_struct("CardElement", 2)?;
                s.serialize_field("tag", "div")?;
                s.serialize_field("text", text)?;
                s.end()
            }
            CardElement::Fields(fields) => {
                let mut s = ser.serialize_struct("CardElement", 2)?;
                s.serialize_field("tag", "div")?;
                s.serialize_field("fields", fields)?;
                s.end()
            }
            CardElement::Button {
                text,
                r#type,
                behaviors,
            } => {
                let mut s = ser.serialize_struct("CardElement", 4)?;
                s.serialize_field("tag", "button")?;
                s.serialize_field("text", text)?;
                s.serialize_field("type", r#type.as_str())?;
                s.serialize_field("behaviors", behaviors)?;
                s.end()
            }
            CardElement::CollapsiblePanel(panel) => {
                let mut s = ser.serialize_struct("CardElement", 7)?;
                s.serialize_field("tag", "collapsible_panel")?;
                s.serialize_field("expanded", &panel.expanded)?;
                s.serialize_field("header", &panel.header)?;
                s.serialize_field("elements", &panel.elements)?;
                // 与官方示例一致的浅边框 + 内边距，让折叠面板在卡片里可辨。
                s.serialize_field(
                    "border",
                    &serde_json::json!({ "color": "grey", "corner_radius": "5px" }),
                )?;
                s.serialize_field("vertical_spacing", "8px")?;
                s.serialize_field("padding", "8px 8px 8px 8px")?;
                s.end()
            }
            CardElement::Form { name, elements } => {
                let mut s = ser.serialize_struct("CardElement", 3)?;
                s.serialize_field("tag", "form")?;
                s.serialize_field("name", name)?;
                s.serialize_field("elements", elements)?;
                s.end()
            }
            CardElement::SelectStatic {
                name,
                placeholder,
                options,
                initial,
                on_change,
            } => {
                // Top-level select_static mirrors the form-container variant
                // from `feishu::forms` (tag, placeholder, options, behaviors,
                // initial_value, name) so on-change callbacks route the
                // selected value through `action.form_value[name]`.
                let opts: Vec<Value> = options
                    .iter()
                    .map(|(value, label)| {
                        serde_json::json!({
                            "text": {"tag": "plain_text", "content": label},
                            "value": value,
                        })
                    })
                    .collect();
                let initial_value: Option<Vec<String>> = initial
                    .as_ref()
                    .filter(|v| !v.is_empty())
                    .map(|v| vec![v.clone()]);
                let mut s = ser.serialize_struct("CardElement", 6)?;
                s.serialize_field("tag", "select_static")?;
                s.serialize_field("placeholder", placeholder)?;
                s.serialize_field("options", &opts)?;
                s.serialize_field(
                    "behaviors",
                    &[CardBehavior {
                        r#type: "callback".into(),
                        value: on_change.clone(),
                    }],
                )?;
                s.serialize_field("name", name)?;
                if let Some(iv) = initial_value {
                    s.serialize_field("initial_value", &iv)?;
                }
                s.end()
            }
        }
    }
}

impl Card {
    pub fn new(title: &str, template: &str) -> Self {
        Self {
            schema: "2.0".into(),
            header: CardHeader {
                title: CardTitle {
                    content: title.into(),
                    tag: "plain_text".into(),
                },
                template: template.into(),
            },
            body: CardBody { elements: vec![] },
        }
    }

    pub fn push_text(&mut self, content: impl Into<String>) {
        self.body.elements.push(CardElement::Markdown {
            content: content.into(),
        });
    }

    pub fn push_note(&mut self, content: impl Into<String>) {
        self.body.elements.push(CardElement::Div {
            text: DivText {
                tag: "plain_text".into(),
                content: content.into(),
                text_size: Some("notation".into()),
                text_color: Some("grey".into()),
            },
        });
    }

    pub fn push_divider(&mut self) {
        self.body.elements.push(CardElement::Hr);
    }

    /// Push callback buttons as first-class V2 body elements. Card JSON 2.0
    /// removed the V1 `action` container (schema-2.0 cards get error 200861
    /// "unsupported tag action" from the API), so each button is its own
    /// body element — they stack vertically, full width — and the click
    /// payload rides in `behaviors: [{type: "callback", value}]`.
    pub fn push_actions(&mut self, actions: Vec<CardButton>) {
        for b in actions {
            self.body.elements.push(CardElement::Button {
                text: b.text,
                r#type: b.r#type,
                behaviors: vec![CardBehavior {
                    r#type: "callback".into(),
                    value: b.value,
                }],
            });
        }
    }
}

/// 卡片 header 标题的字符上限：主题过长即截断加省略号，防超飞书标题限长。
const CARD_TITLE_MAX_CHARS: usize = 40;

/// 从用户首条 prompt 派生卡片主题（header title）。
///
/// 逐行清理后取首个非空行（`clean_topic_line`）。清理规则按顺序：
/// 1. 纯代码围栏行（```` ``` ```` 或 ```` ```lang ````）整行跳过，看下一行。
/// 2. 嵌套块引用：剥掉所有前导 `>` 和之后的空白（`>>` / `>>>` 一并处理）。
/// 3. 行首/行尾 markdown 标题标记（`#` / `##` / `###` …）各剥一次。
/// 4. 行首/行尾内联反引号各剥一次（避免 ```` `cargo build` 跑一下 ```` 留下
///    游离 `` ` ``；中间的反引号保留）。
///
/// 全部行清理后仍为空（空 prompt / lazy seed / 纯围栏）回退中性占位
/// `"Claude Code"`。超长截断成 `…`。`user_prompt` 会话内不变（root 卡只在
/// spawn 时种一次，flush 不重写），故标题随会话保持稳定，跨卡 / 换卡一致
/// —— 正表达"主题"语义。
pub fn derive_topic(prompt: &str) -> String {
    let first_line = prompt.lines().map(clean_topic_line).find(|l| !l.is_empty());
    let Some(cleaned) = first_line else {
        return "Claude Code".to_string();
    };
    let mut out: String = cleaned.chars().take(CARD_TITLE_MAX_CHARS).collect();
    if cleaned.chars().count() > CARD_TITLE_MAX_CHARS {
        out.push('…');
    }
    out
}

/// 单行 prompt 清理：剥掉会让标题变难读的 markdown 噪音。
///
/// 顺序很关键 —— 先看是否围栏行（不剥围栏符以外的东西），再剥嵌套引用 / 标题
/// 标记 / 行首尾内联反引号，最后 trim 一次。所有规则都是 best-effort：不试图
/// 解析完整 markdown，只把常见噪音剥掉。
fn clean_topic_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // 1. 纯代码围栏行（``` 或 ```lang）整行跳过：返回空串让调用方看下一行。
    //    在其他清理之前判断，避免被 `>` / `#` 规则误吃。
    if trimmed.starts_with("```") {
        return String::new();
    }
    let mut s = trimmed.to_string();
    // 2. 嵌套块引用：剥掉所有前导 `>` 和之后的空白。
    while let Some(rest) = s.strip_prefix('>') {
        s = rest.trim_start().to_string();
        if s.is_empty() {
            break;
        }
    }
    // 3. 行首/行尾 markdown 标题标记各剥一次（`#` / `##` / `###` 都吃）。
    s = s.trim_start_matches('#').trim_start().to_string();
    // 4. 行首/行尾内联反引号各剥一次，避免游离 `` ` ``。
    if let Some(rest) = s.strip_prefix('`') {
        s = rest.to_string();
    }
    if let Some(rest) = s.strip_suffix('`') {
        s = rest.to_string();
    }
    s.trim().to_string()
}

/// 卡片 footer 的可选信息：替代 `msg_id: {session_id}` 灰注，展示模型名和
/// token 用量，对用户更有用。
#[derive(Debug, Clone, Default)]
pub struct CardFooter {
    /// 模型名，如 "claude-sonnet-4-20250514"
    pub model: Option<String>,
    /// 本轮输入 token 数
    pub round_input: u64,
    /// 本轮输出 token 数
    pub round_output: u64,
    /// 累计输入 token 数（整个会话）
    pub total_input: u64,
    /// 累计输出 token 数（整个会话）
    pub total_output: u64,
}

/// 格式化 token 数为人类可读（<1000 显示原数，>=1000 显示 K 单位）。
fn format_token_count(n: u64) -> String {
    if n >= 1000 {
        let k = n as f64 / 1000.0;
        if k >= 100.0 {
            format!("{:.0}K", k)
        } else {
            format!("{:.1}K", k)
        }
    } else {
        n.to_string()
    }
}

/// 从模型名中提取简短标识（去掉版本后缀），如 "claude-sonnet-4-20250514" → "sonnet"。
fn short_model_name(model: &str) -> String {
    // Known model prefixes: "claude-sonnet-4-…" → "sonnet", "claude-opus-4-…" → "opus"
    let known = [
        ("claude-sonnet-4", "sonnet"),
        ("claude-sonnet-5", "sonnet"),
        ("claude-opus-4", "opus"),
        ("claude-opus-5", "opus"),
        ("claude-haiku-3", "haiku"),
        ("claude-haiku-4", "haiku"),
    ];
    for (prefix, short) in &known {
        if model.starts_with(prefix) {
            return short.to_string();
        }
    }
    // Fallback: take the second segment if it looks like "claude-{name}"
    if let Some(rest) = model.strip_prefix("claude-") {
        if let Some(name) = rest.split('-').next() {
            return name.to_string();
        }
    }
    model.to_string()
}

/// 从累积状态构建完整卡（spec §4.3）：
/// header(`{主题}`, theme) + 引用块(`> {user_prompt}`) + 分隔线
/// + body 各元素 + footer 灰注。
/// 标题由 `derive_topic(user_prompt)` 派生（首条非空 prompt 行），不再携带
/// 状态 emoji；状态由 Feishu reaction 表达（见 `router::card_state::phase`）。
/// 当 `footer` 为 Some 时展示模型名和 token 用量，否则回退 `msg_id: {session_id}`。
///
/// Footer 三段语义（避免把 in/out 当 round、ctx 当 in+out 求和的常见错）：
/// - `in`  = 累计输入 token（整个会话的 prompt 累计，作为对话"成本"）
/// - `out` = 累计输出 token（整个会话的生成累计）
/// - `ctx` = 当前上下文窗口 token（=累计输入，因为输入就是填入上下文的全部
///   内容；不应把 in+out 求和，那只是总流量不是上下文）
pub fn render_accumulated_card(
    user_prompt: &str,
    session_id: &str,
    body: &[CardElement],
    theme: &str,
    footer: Option<&CardFooter>,
) -> Card {
    let mut card = Card::new(&derive_topic(user_prompt), theme);
    card.push_text(format!("> {user_prompt}"));
    card.push_divider();
    for el in body {
        card.body.elements.push(el.clone());
    }
    match footer {
        Some(f) => {
            let model = f
                .model
                .as_deref()
                .map(short_model_name)
                .unwrap_or_else(|| "?".to_string());
            let total_in = format_token_count(f.total_input);
            let total_out = format_token_count(f.total_output);
            let ctx = format_token_count(f.total_input);
            card.push_note(format!(
                "{model}  ·  in: {total_in}  out: {total_out}  ·  ctx: {ctx}"
            ));
        }
        None => {
            card.push_note(format!("msg_id: {session_id}"));
        }
    }
    card
}

/// seed 时的初始卡构建器（不再被每个事件调用）。空 body 薄封装。
/// 保留供 cards_test 快照；theme 固定 "blue" 以保持快照不变。
pub fn render_root_card(user_prompt: &str, msg_id: &str) -> Card {
    render_accumulated_card(user_prompt, msg_id, &[], "blue", None)
}

/// 权限卡参数展示的代码级硬上限：完整参数（含折叠面板里的 JSON）超过即截断，
/// 防止整卡超过飞书卡片消息体 30KB 上限导致发卡失败、权限流程卡死。
const PERMISSION_ARGS_HARD_LIMIT_CHARS: usize = 8192;

/// 单行字段值的预览长度；长文本（Write content、Edit old/new）只显示开头，
/// 完整内容由折叠面板收纳。
const PERMISSION_FIELD_PREVIEW_CHARS: usize = 300;

/// 常见参数的展示标签（B 方案字段行用），未知 key 保留原名。
fn permission_field_label(key: &str) -> &str {
    match key {
        "command" | "cmd" => "命令",
        "file_path" | "path" => "路径",
        "pattern" => "模式",
        "url" => "链接",
        "prompt" => "提示",
        "query" => "查询",
        "offset" => "起始行",
        "limit" => "读取行数",
        "timeout" => "超时",
        "description" => "描述",
        "restart" => "重启终端",
        "replace_all" => "全部替换",
        "old_string" => "原文",
        "new_string" => "替换为",
        "content" => "内容",
        "session_id" => "会话",
        _ => key,
    }
}

/// 按字符数截取（UTF-8 安全），超长追加省略号。
fn preview_chars(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        s.to_string()
    } else {
        let cut: String = s.chars().take(limit).collect();
        format!("{cut}…")
    }
}

/// 标量值的展示文本；对象/数组只出现在回退路径，不会走到这里。
fn scalar_display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "—".into(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "…".into()),
    }
}

/// 扁平对象：所有值都是标量（string/number/bool/null），适合字段行展示。
fn is_flat_object(args: &Value) -> bool {
    matches!(
        args,
        Value::Object(map)
            if map.values().all(|v| matches!(
                v,
                Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
            ))
    )
}

/// 是否有字段值长到需要行内预览 + 折叠完整参数。
fn has_long_value(args: &Value) -> bool {
    args.as_object()
        .map(|map| {
            map.values().any(|v| {
                matches!(v, Value::String(s) if s.chars().count() > PERMISSION_FIELD_PREVIEW_CHARS)
            })
        })
        .unwrap_or(false)
}

/// Bash 命令摘要行：短命令用行内代码（`$ cmd`），多行/超长用 bash 代码块。
fn command_headline(cmd: &str) -> CardElement {
    let capped = preview_chars(cmd, PERMISSION_ARGS_HARD_LIMIT_CHARS);
    let content = if capped.contains('\n') || capped.chars().count() > 160 {
        format!("```bash\n{capped}\n```")
    } else {
        format!("`$ {capped}`")
    };
    CardElement::Markdown { content }
}

/// 把 args 里没被摘要行消费的 key 渲染成 div.fields 行（B 方案）。
/// `skip` 里的 key 已出现在摘要行，不再重复。返回 None 表示没有可展示字段。
fn args_field_rows(args: &Value, skip: &[&str]) -> Option<CardElement> {
    let map = args.as_object()?;
    let fields: Vec<CardField> = map
        .iter()
        .filter(|(k, _)| !skip.contains(&k.as_str()))
        .map(|(k, v)| CardField {
            is_short: false,
            text: CardText {
                tag: "lark_md".into(),
                content: format!(
                    "**{}**\n{}",
                    permission_field_label(k),
                    preview_chars(&scalar_display(v), PERMISSION_FIELD_PREVIEW_CHARS)
                ),
            },
        })
        .collect();
    if fields.is_empty() {
        return None;
    }
    Some(CardElement::Fields(fields))
}

/// 折叠面板：收纳完整参数 JSON（默认收起），受硬上限保护，截断时附灰注。
fn full_args_panel(args: &Value) -> CardElement {
    let pretty = serde_json::to_string_pretty(args).unwrap_or_default();
    let capped = preview_chars(&pretty, PERMISSION_ARGS_HARD_LIMIT_CHARS);
    let mut elements = vec![CardElement::Markdown {
        content: format!("```json\n{capped}\n```"),
    }];
    if capped.chars().count() < pretty.chars().count() {
        elements.push(CardElement::Div {
            text: DivText {
                tag: "plain_text".into(),
                content: "（参数过长，已截断）".into(),
                text_size: Some("notation".into()),
                text_color: Some("grey".into()),
            },
        });
    }
    CardElement::CollapsiblePanel(CollapsiblePanel {
        expanded: false,
        header: CollapsiblePanelHeader {
            title: CardText {
                tag: "plain_text".into(),
                content: "完整参数".into(),
            },
            icon: StandardIcon {
                tag: "standard_icon".into(),
                token: "down-small-ccm_outlined".into(),
                size: "16px 16px".into(),
            },
            icon_position: "right".into(),
            icon_expanded_angle: -180,
        },
        elements,
    })
}

/// 把工具调用参数渲染成易读元素序列（方案 A+B）：
///
/// - Bash 命令独立成摘要行（`$ ...`），其余参数 field 行；
/// - 文件/链接类工具路径或 URL 摘要行 + 其余 field 行；
/// - 长文本字段行内预览，完整参数收进折叠面板；
/// - 未知/嵌套参数回退 pretty JSON 代码块（原行为）。
///
/// 所有输出受硬上限保护，整卡不会超飞书 30KB。
fn permission_args_elements(tool_name: &str, args: &Value) -> Vec<CardElement> {
    // Bash：命令是放行决策的关键，独立成行完整展示。
    if tool_name.eq_ignore_ascii_case("bash")
        && let Some(cmd) = args
            .get("command")
            .or_else(|| args.get("cmd"))
            .and_then(Value::as_str)
    {
        let mut els = vec![command_headline(cmd)];
        if let Some(rows) = args_field_rows(args, &["command", "cmd"]) {
            els.push(rows);
        }
        return els;
    }
    if is_flat_object(args) {
        let mut els = vec![];
        if let Some(p) = args
            .get("file_path")
            .or_else(|| args.get("path"))
            .and_then(Value::as_str)
        {
            els.push(CardElement::Markdown {
                content: format!("📄 `{}`", preview_chars(p, PERMISSION_FIELD_PREVIEW_CHARS)),
            });
            if let Some(rows) = args_field_rows(args, &["file_path", "path"]) {
                els.push(rows);
            }
        } else if let Some(u) = args.get("url").and_then(Value::as_str) {
            els.push(CardElement::Markdown {
                content: format!("🌐 `{}`", preview_chars(u, PERMISSION_FIELD_PREVIEW_CHARS)),
            });
            if let Some(rows) = args_field_rows(args, &["url"]) {
                els.push(rows);
            }
        } else if let Some(rows) = args_field_rows(args, &[]) {
            els.push(rows);
        }
        if has_long_value(args) {
            els.push(full_args_panel(args));
        }
        if !els.is_empty() {
            return els;
        }
    }
    // 嵌套/未知/空参数：回退 pretty JSON 代码块（原行为），仍受硬上限保护。
    let pretty = serde_json::to_string_pretty(args).unwrap_or_default();
    let capped = preview_chars(&pretty, PERMISSION_ARGS_HARD_LIMIT_CHARS);
    vec![CardElement::Markdown {
        content: format!("```json\n{capped}\n```"),
    }]
}

pub fn render_permission_card(
    session_id: &str,
    request_id: &str,
    tool_name: &str,
    args: &Value,
) -> Card {
    let mut card = Card::new("⚠ 权限请求", "orange");
    card.push_text(format!("**{tool_name}** 想要执行："));
    // A+B 方案：常见工具渲染成一行摘要 + 字段行，长参数折叠收纳；
    // 未知/嵌套参数回退 pretty JSON 代码块。
    for el in permission_args_elements(tool_name, args) {
        card.body.elements.push(el);
    }
    card.push_note("本会话不再询问 = 之后本会话所有权限请求自动放行；/new 或会话结束后失效");
    let btn = |label: &str, kind: &str, decision: &str| CardButton {
        text: CardText {
            tag: "plain_text".into(),
            content: label.into(),
        },
        r#type: kind.into(),
        value: serde_json::json!({
            "session_id": session_id,
            "request_id": request_id,
            "decision": decision,
        }),
    };
    card.push_actions(vec![
        btn("本次允许", "primary", "allow_once"),
        btn("本会话不再询问", "default", "allow_session"),
        btn("拒绝", "danger", "deny"),
    ]);
    card
}

/// Card shown when a button callback references a session that no longer
/// exists (process exited / daemon restarted without it).
pub fn render_dead_session_card() -> Card {
    let mut card = Card::new("会话已结束", "grey");
    card.push_text("会话已结束，请发送 /new 开启新的会话。");
    card
}

/// Shown when a restored mapping's conversation is gone on the claude side
/// (resume rejected: session files cleaned) and sebas transparently fell
/// back to a fresh session (sebas-dk8.4). The old context did NOT carry
/// over — say so instead of silently continuing.
pub fn render_session_lost_card() -> Card {
    let mut card = Card::new("已开启新会话", "orange");
    card.push_text("⚠️ 未找到原会话记录（可能已被清理），本次对话将作为新会话继续。");
    card
}

/// In-place replacement for a permission card after the user clicks an
/// option. `label` is one of "✅ 已允许（仅此一次）" / "✅ 已允许（本会话不再询问）" /
/// "❌ 已拒绝". No buttons — the click is already done.
pub fn render_resolved_permission_card(label: &str) -> Card {
    let mut card = Card::new("权限请求", "blue");
    card.push_text(label);
    card
}

/// Card sent on a stale permission click (the request was already
/// resolved by an earlier click, or the original tool_use never
/// produced a tracked permission card). Tells the user their click
/// had no effect without making the bot look broken.
pub fn render_expired_permission_card() -> Card {
    let mut card = Card::new("权限请求", "grey");
    card.push_text("⚠ 请求已过期，该工具调用已被处理或取消。");
    card
}

/// Command groups for the interactive help card.
/// Maps group key → (tab label, list of (command, description)).
fn help_command_groups() -> HashMap<&'static str, (&'static str, Vec<(&'static str, &'static str)>)>
{
    let mut m: HashMap<&str, (&str, Vec<(&str, &str)>)> = HashMap::new();
    m.insert(
        "session",
        (
            "💬 会话管理",
            vec![
                ("/new", "开新会话"),
                ("/sessions", "列出会话"),
                ("/switch ⟨n⟩", "切换到第 n 个会话"),
                ("/resume ⟨sid⟩", "恢复指定会话"),
                ("/cancel", "中断当前轮"),
                ("/status", "查看当前会话状态"),
                ("/compact", "压缩上下文"),
                ("/cost", "查看会话开销"),
            ],
        ),
    );
    m.insert(
        "system",
        (
            "⚙️ 系统功能",
            vec![
                ("/settings [key [val]]", "查看/修改配置"),
                ("/model ⟨text⟩", "透传 /model 给 claude code"),
                ("/goal ⟨text⟩", "透传 /goal 给 claude code"),
                ("/cd ⟨path⟩", "切换工作目录"),
                ("/btw ⟨text⟩", "插队提问"),
            ],
        ),
    );
    m.insert(
        "service",
        (
            "🔧 服务管理",
            vec![
                ("/upgrade [dev] [dry-run]", "升级并重启"),
                ("/rollback", "回滚并重启"),
                ("/restart", "重启 core"),
                ("/services", "查看服务状态"),
                ("/system", "查看系统状态"),
                ("/gateway on|off|restart|status", "管理 gateway"),
                ("/webui status", "查看 webui 状态"),
            ],
        ),
    );
    m.insert(
        "other",
        (
            "📦 其他",
            vec![("/provider", "管理 Provider"), ("/help", "显示本帮助")],
        ),
    );
    m
}

/// Render an interactive help card with group tabs and clickable command buttons.
///
/// `group` — the currently selected group key ("session" | "system" | "service" | "other").
/// `theme` — card theme color (e.g. "blue").
///
/// The card includes:
/// - A row of tab buttons for switching groups (in-place update via `UpdateCardByMsgId`).
/// - A list of command buttons for the selected group (clicking executes the command).
/// The `msg_id` of the card is stored by the dispatcher so tab switches can PATCH in place.
pub fn render_help_card(group: &str, theme: &str) -> Card {
    let groups = help_command_groups();
    // Validate group key; fall back to "session".
    let group = if groups.contains_key(group) {
        group
    } else {
        "session"
    };
    let (tab_label, commands) = &groups[group];

    let mut card = Card::new(&format!("📖 帮助 — {tab_label}"), theme);

    // Tab buttons row: all groups as buttons, current group highlighted.
    let tab_order = ["session", "system", "service", "other"];
    let mut tab_buttons = Vec::new();
    for key in tab_order.iter() {
        let (label, _) = &groups[key];
        let btn_type = if *key == group { "primary" } else { "default" };
        tab_buttons.push(CardButton {
            text: CardText {
                tag: "plain_text".into(),
                content: (*label).into(),
            },
            r#type: btn_type.into(),
            value: serde_json::json!({"help_tab": key}),
        });
    }
    card.push_actions(tab_buttons);

    // Divider + command buttons for the selected group
    card.push_divider();
    for (cmd, desc) in commands {
        card.body.elements.push(CardElement::Button {
            text: CardText {
                tag: "plain_text".into(),
                content: format!("{cmd}  —  {desc}"),
            },
            r#type: "default".into(),
            behaviors: vec![CardBehavior {
                r#type: "callback".into(),
                value: serde_json::json!({"help_cmd": cmd}),
            }],
        });
    }

    card.push_note("提示：点击分组 tab 切换，点击命令直接执行");
    card
}

/// Shown when the agent fails to spawn or times out during handshake.
pub fn render_error_card(message: &str) -> Card {
    let mut card = Card::new("❌ 启动失败", "red");
    // If message spans multiple lines or has structured content, render in
    // a code fence so it doesn't reflow awkwardly as inline text.
    if message.contains('\n') || message.len() > 120 {
        card.push_text(format!("```\n{message}\n```"));
    } else {
        card.push_text(message.to_string());
    }
    card
}
