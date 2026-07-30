use acp_claude::session::AcpEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "tag", rename_all = "snake_case")]
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
    /// was removed); callback payloads travel via `behaviors`.
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

#[derive(Debug, Clone, Serialize)]
pub struct CardButton {
    pub text: CardText,
    pub r#type: String, // "primary" | "danger" | "default"
    pub value: Value,
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

    pub fn push_actions(&mut self, actions: Vec<CardButton>) {
        for a in actions {
            self.body.elements.push(CardElement::Button {
                text: a.text,
                r#type: a.r#type,
                behaviors: vec![CardBehavior {
                    r#type: "callback".into(),
                    value: a.value,
                }],
            });
        }
    }
}

pub fn render_root_card(user_prompt: &str, msg_id: &str, status_emoji: &str) -> Card {
    let mut card = Card::new(&format!("{status_emoji} Claude Code"), "blue");
    card.push_text(format!("> {user_prompt}"));
    card.push_divider();
    card.push_note(format!("msg_id: {msg_id}"));
    card
}

pub fn render_permission_card(
    session_id: &str,
    request_id: &str,
    tool_name: &str,
    args: &Value,
) -> Card {
    let mut card = Card::new("⚠ 权限请求", "orange");
    card.push_text(format!("**{tool_name}** 想要执行："));
    card.push_note(serde_json::to_string_pretty(args).unwrap_or_default());
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
        btn("Allow once", "primary", "allow_once"),
        btn("Allow session", "default", "allow_session"),
        btn("Deny", "danger", "deny"),
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

/// Shown when the agent fails to spawn or times out during handshake.
pub fn render_error_card(message: &str) -> Card {
    let mut card = Card::new("❌ 启动失败", "red");
    card.push_text(message.to_string());
    card
}

pub fn apply_event(card: &mut Card, event: &AcpEvent) {
    match event {
        AcpEvent::TextDelta { delta, .. } => card.push_text(delta.clone()),
        AcpEvent::ThinkingDelta { delta, .. } => card.push_note(format!("💭 {delta}")),
        AcpEvent::ToolStart {
            tool_name, args, ..
        } => {
            card.push_divider();
            card.push_text(format!("📖 **{tool_name}** `{}`", args));
        }
        AcpEvent::ToolEnd {
            tool_name, result, ..
        } => {
            card.push_note(format!("✓ {tool_name} done: {}", truncate(result, 200)));
        }
        AcpEvent::PermissionRequest {
            tool_name, args, ..
        } => {
            card.push_text(format!("⏸ waiting for permission: {tool_name} `{}`", args));
        }
        AcpEvent::Finished { .. } => card.push_text("✅ 完成"),
        AcpEvent::Error { message, .. } => card.push_text(format!("❌ {message}")),
        AcpEvent::ToolProgress {
            tool_name,
            progress,
            ..
        } => {
            card.push_note(format!("⏳ {tool_name}: {progress}"));
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    }
}
