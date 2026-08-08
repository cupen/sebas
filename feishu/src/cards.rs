use acp_claude::session::AcpEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Map a router FSM phase string (the Feishu `emoji_type`, e.g. `"Typing"`,
/// `"OnIt"`, `"DONE"`, `"CrossMark"`) to the Unicode glyph shown in the card
/// header. Feishu rejects arbitrary Unicode as reaction `emoji_type`
/// (error 231001), so the state stores the API name and we render the visual
/// separately. Unknown phases fall back to a neutral bullet so a bad value
/// can't break the card.
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
#[derive(Debug, Clone, Deserialize)]
pub struct CardConfig {
    #[serde(default = "default_theme_color")]
    pub theme_color: String,
    #[serde(default = "default_max_user_text")]
    pub max_user_text_chars: usize,
    #[serde(default = "default_max_tool_output")]
    pub max_tool_output_chars: usize,
    #[serde(default = "default_true")]
    pub fold_long_output: bool,
}

impl Default for CardConfig {
    fn default() -> Self {
        Self {
            theme_color: default_theme_color(),
            max_user_text_chars: default_max_user_text(),
            max_tool_output_chars: default_max_tool_output(),
            fold_long_output: true,
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
    2000
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

/// 从累积状态构建完整卡（spec §4.3）：
/// header(`{emoji} Claude Code`, theme) + 引用块(`> {user_prompt}`) + 分隔线
/// + body 各元素 + footer 灰注(`msg_id: {session_id}`)。
pub fn render_accumulated_card(
    user_prompt: &str,
    session_id: &str,
    status_emoji: &str,
    body: &[CardElement],
    theme: &str,
) -> Card {
    let mut card = Card::new(&format!("{status_emoji} Claude Code"), theme);
    card.push_text(format!("> {user_prompt}"));
    card.push_divider();
    for el in body {
        card.body.elements.push(el.clone());
    }
    card.push_note(format!("msg_id: {session_id}"));
    card
}

/// seed 时的初始卡构建器（不再被每个事件调用）。空 body 薄封装。
/// 保留供 cards_test 快照；theme 固定 "blue" 以保持快照不变。
pub fn render_root_card(user_prompt: &str, msg_id: &str, status_emoji: &str) -> Card {
    render_accumulated_card(user_prompt, msg_id, status_emoji, &[], "blue")
}

pub fn render_permission_card(
    session_id: &str,
    request_id: &str,
    tool_name: &str,
    args: &Value,
) -> Card {
    let mut card = Card::new("⚠ 权限请求", "orange");
    card.push_text(format!("**{tool_name}** 想要执行："));
    // Render args as a fenced JSON code block so Feishu gives it a
    // scrollable code-style container instead of a grey note div that
    // looks like a wall of JSON.
    let args_str = serde_json::to_string_pretty(args).unwrap_or_default();
    card.push_text(format!("```json\n{args_str}\n```"));
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

/// 把一个事件累积进 body（spec §4.2/§7）。复活 ThinkingDelta/ToolEnd/ToolProgress。
/// 单元素截断（max_user_text_chars / max_tool_output_chars + fold_long_output）
/// + 总量兜底（24000 丢旧，Hr 连后一个一起丢）。PermissionRequest 不累积（走独立 SendCard）。
pub fn apply_event_to_card(body: &mut Vec<CardElement>, event: &AcpEvent, cfg: &CardConfig) {
    match event {
        AcpEvent::TextDelta { delta, .. } => {
            push_text_truncated(body, delta, cfg.max_user_text_chars, cfg.fold_long_output);
        }
        AcpEvent::ThinkingDelta { delta, .. } => {
            // Visual separator before the thinking note so it's distinct
            // from the previous event's text output.
            body.push(CardElement::Hr);
            body.push(note_element(format!("💭 {delta}")));
        }
        AcpEvent::ToolStart {
            tool_name, args, ..
        } => {
            body.push(CardElement::Hr);
            // Tool args in a fenced JSON code block — readable for nested
            // objects/arrays, vs inline backtick which collapses to one line.
            let args_str = serde_json::to_string_pretty(args).unwrap_or_default();
            push_text_truncated(
                body,
                &format!("📖 **{tool_name}**\n```json\n{args_str}\n```"),
                cfg.max_user_text_chars,
                cfg.fold_long_output,
            );
        }
        AcpEvent::ToolEnd {
            tool_name, result, ..
        } => {
            let (text, note) =
                truncate_with_note(result, cfg.max_tool_output_chars, cfg.fold_long_output);
            body.push(note_element(format!("✓ {tool_name} done: {text}")));
            if let Some(n) = note {
                body.push(note_element(format!("（已折叠 {n} 字）")));
            }
        }
        AcpEvent::ToolProgress {
            tool_name,
            progress,
            ..
        } => {
            body.push(note_element(format!("⏳ {tool_name}: {progress}")));
        }
        AcpEvent::Finished { .. } => body.push(CardElement::Markdown {
            content: "✅ 完成".into(),
        }),
        AcpEvent::Error { message, .. } => body.push(CardElement::Markdown {
            content: format!("❌ {message}"),
        }),
        AcpEvent::PermissionRequest { .. } => {} // 独立 SendCard，不累积
    }
    enforce_total_budget(body, cfg);
}

/// 截断文本到 `limit` 字符；超限则返回 (截断文本, Some(溢出字符数))。
fn truncate_with_note(s: &str, limit: usize, fold: bool) -> (String, Option<usize>) {
    if !fold {
        return (s.to_string(), None);
    }
    let count = s.chars().count();
    if count <= limit {
        return (s.to_string(), None);
    }
    let truncated: String = s.chars().take(limit).collect();
    (truncated, Some(count - limit))
}

/// push 一段 Markdown 文本，必要时截断 + 追加灰注。
fn push_text_truncated(body: &mut Vec<CardElement>, text: &str, limit: usize, fold: bool) {
    let (content, note) = truncate_with_note(text, limit, fold);
    body.push(CardElement::Markdown { content });
    if let Some(n) = note {
        body.push(note_element(format!("（已折叠 {n} 字）")));
    }
}

/// 构造一个灰注 Div 元素（notation size + grey）。
fn note_element(content: String) -> CardElement {
    CardElement::Div {
        text: DivText {
            tag: "plain_text".into(),
            content,
            text_size: Some("notation".into()),
            text_color: Some("grey".into()),
        },
    }
}

/// 总量兜底（spec §7）：body 累积字符 > 24000 -> 丢最旧；最旧是 Hr 则连后一个一起丢。
fn enforce_total_budget(body: &mut Vec<CardElement>, _cfg: &CardConfig) {
    const TOTAL_BUDGET: usize = 24000;
    while total_chars(body) > TOTAL_BUDGET {
        if body.is_empty() {
            break;
        }
        // 最旧是 Hr -> 连后一个一起丢（不留悬空分隔线）。
        let drop_two = matches!(body.first(), Some(CardElement::Hr));
        body.remove(0);
        if drop_two && !body.is_empty() {
            body.remove(0);
        }
    }
}

fn total_chars(body: &[CardElement]) -> usize {
    body.iter()
        .map(|e| match e {
            CardElement::Markdown { content } => content.chars().count(),
            CardElement::Div { text } => text.content.chars().count(),
            _ => 0,
        })
        .sum()
}
