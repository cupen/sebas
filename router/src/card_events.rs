//! AcpEvent → 卡片元素的累积翻译（spec §4.2/§7）。
//!
//! 属于业务编排层：feishu crate 只持有纯卡片协议类型（`CardElement` 等），
//! 事件到卡片的翻译放在 router，避免 feishu 反向依赖 acp-claude。

use acp_claude::session::AcpEvent;
use feishu::cards::{CardConfig, CardElement, DivText};

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
