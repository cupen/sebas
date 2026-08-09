//! AcpEvent → 卡片元素的累积翻译（spec §4.2/§7）。
//!
//! 属于业务编排层：feishu crate 只持有纯卡片协议类型（`CardElement` 等），
//! 事件到卡片的翻译放在 router，避免 feishu 反向依赖 acp-claude。

use acp_claude::session::AcpEvent;
use feishu::cards::{
    CardConfig, CardElement, CardText, CollapsiblePanel, CollapsiblePanelHeader, DivText,
    StandardIcon, ThinkingDisplay,
};

/// 把一个事件累积进 body（spec §4.2/§7）。复活 ThinkingDelta/ToolEnd/ToolProgress。
/// fold_long_output=true 时：ToolStart 折叠成一个 collapsible_panel（默认收起），
/// ToolProgress/ToolEnd 都收进对应工具面板，卡片里每个工具只占一行；
/// tool result 默认屏蔽（max_tool_output_chars=0），>0 时结果也收进面板。
/// fold_long_output=false 时：保持内联展示（结果仍受 10240 硬上限保护）。
/// + 总量兜底（24000 丢旧，Hr 连后一个一起丢）。PermissionRequest 不累积（走独立 SendCard）。
pub fn apply_event_to_card(body: &mut Vec<CardElement>, event: &AcpEvent, cfg: &CardConfig) {
    match event {
        AcpEvent::TextDelta { delta, .. } => {
            push_text_truncated(body, delta, cfg.max_user_text_chars, cfg.fold_long_output);
        }
        AcpEvent::ThinkingDelta { delta, .. } => {
            if cfg.thinking == ThinkingDisplay::Hide {
                // 完全丢弃：模型仍在思考，只是卡片不展示。
            } else {
                append_thinking_delta(body, delta);
            }
        }
        AcpEvent::ToolStart {
            tool_name, args, ..
        } => {
            // Tool args in a fenced JSON code block — readable for nested
            // objects/arrays, vs inline backtick which collapses to one line.
            let args_str = serde_json::to_string_pretty(args).unwrap_or_default();
            if cfg.fold_long_output {
                // 每个工具一个默认折叠的面板，展开才看得到 args。
                body.push(CardElement::CollapsiblePanel(CollapsiblePanel {
                    expanded: false,
                    header: panel_header(format!("📖 {tool_name}")),
                    elements: text_with_truncation_note(
                        format!("```json\n{args_str}\n```"),
                        cfg.max_user_text_chars,
                        true,
                    ),
                }));
            } else {
                body.push(CardElement::Hr);
                push_text_truncated(
                    body,
                    &format!("📖 **{tool_name}**\n```json\n{args_str}\n```"),
                    cfg.max_user_text_chars,
                    cfg.fold_long_output,
                );
            }
        }
        AcpEvent::ToolEnd {
            tool_name, result, ..
        } => {
            if cfg.fold_long_output {
                if let Some(panel) = last_tool_panel_mut(body, tool_name) {
                    panel.header.title.content = format!("✓ {tool_name}");
                    panel.elements.extend(tool_result_elements(tool_name, result, cfg));
                    return;
                }
                // fold 模式下没有可归属的面板（异常时序）：静默，不输出结果。
            } else {
                push_tool_end_result(body, tool_name, result, cfg);
            }
        }
        AcpEvent::ToolProgress {
            tool_name,
            progress,
            ..
        } => {
            if let Some(panel) = last_tool_panel_mut(body, tool_name) {
                panel.elements.push(note_element(format!("⏳ {progress}")));
            } else {
                body.push(note_element(format!("⏳ {tool_name}: {progress}")));
            }
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

/// 生成一段 Markdown 文本元素，必要时截断 + 追加灰注（供面板内使用）。
fn text_with_truncation_note(text: String, limit: usize, fold: bool) -> Vec<CardElement> {
    let (content, note) = truncate_with_note(&text, limit, fold);
    let mut elements = vec![CardElement::Markdown { content }];
    if let Some(n) = note {
        elements.push(note_element(format!("（已折叠 {n} 字）")));
    }
    elements
}

/// push 一段 Markdown 文本，必要时截断 + 追加灰注。
fn push_text_truncated(body: &mut Vec<CardElement>, text: &str, limit: usize, fold: bool) {
    body.extend(text_with_truncation_note(text.to_string(), limit, fold));
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

/// ToolEnd 结果渲染（fold_long_output=false 的内联路径）：
/// - `max_tool_output_chars == 0`：完全不输出 tool call 的结果内容；
/// - 否则灰注内联展示，超过 10240 硬上限才截断 + 灰注。
fn push_tool_end_result(
    body: &mut Vec<CardElement>,
    tool_name: &str,
    result: &str,
    cfg: &CardConfig,
) {
    if cfg.max_tool_output_chars == 0 {
        return;
    }
    let content = cap_chars(result, TOOL_RESULT_HARD_LIMIT_CHARS);
    let overflow = result
        .chars()
        .count()
        .saturating_sub(content.chars().count());
    body.push(note_element(format!("✓ {tool_name} done: {content}")));
    if overflow > 0 {
        body.push(note_element(format!("（已截断 {overflow} 字）")));
    }
}

/// 生成 tool result 的展示元素（fold 模式下收进对应工具面板）：
/// - `max_tool_output_chars == 0`：不输出结果内容（返回空）；
/// - 结果 <= 软上限：灰注 `✓ {tool} done: ...`；
/// - 结果 > 软上限：markdown 全文保留（多行用代码围栏），超出 10240 硬上限才截断。
fn tool_result_elements(tool_name: &str, result: &str, cfg: &CardConfig) -> Vec<CardElement> {
    if cfg.max_tool_output_chars == 0 {
        return vec![];
    }
    let content = cap_chars(result, TOOL_RESULT_HARD_LIMIT_CHARS);
    let overflow = result
        .chars()
        .count()
        .saturating_sub(content.chars().count());
    let mut elements = vec![];
    if result.chars().count() > cfg.max_tool_output_chars {
        // 长结果：多行用代码围栏包住，避免在 markdown 里被重排。
        if content.contains('\n') {
            elements.push(CardElement::Markdown {
                content: format!("```\n{content}\n```"),
            });
        } else {
            elements.push(CardElement::Markdown {
                content: content.clone(),
            });
        }
    } else {
        elements.push(note_element(format!("✓ {tool_name} done: {content}")));
    }
    if overflow > 0 {
        elements.push(note_element(format!("（已截断 {overflow} 字）")));
    }
    elements
}

/// 取 body 末尾属于 `tool_name` 的折叠面板（标题形如 `📖 Bash` / `⏳ Bash`）。
/// 事件按顺序到达（ToolStart → ToolProgress* → ToolEnd），末尾即当前工具的面板。
fn last_tool_panel_mut<'a>(
    body: &'a mut [CardElement],
    tool_name: &str,
) -> Option<&'a mut CollapsiblePanel> {
    let suffix = format!(" {tool_name}");
    match body.last_mut()? {
        CardElement::CollapsiblePanel(panel) if panel.header.title.content.ends_with(&suffix) => {
            Some(panel)
        }
        _ => None,
    }
}

/// 折叠面板的标准 header（右侧箭头图标，展开时旋转 180°）。
fn panel_header(title: String) -> CollapsiblePanelHeader {
    CollapsiblePanelHeader {
        title: CardText {
            tag: "plain_text".into(),
            content: title,
        },
        icon: StandardIcon {
            tag: "standard_icon".into(),
            token: "down-small-ccm_outlined".into(),
            size: "16px 16px".into(),
        },
        icon_position: "right".into(),
        icon_expanded_angle: -180,
    }
}

/// ThinkingDelta 折叠面板的 header（复用 panel_header 的标准图标）。
fn thinking_panel_header() -> CollapsiblePanelHeader {
    panel_header("💭 思考".into())
}

/// 把 ThinkingDelta 累积进尾部 thinking 面板；body 末尾不是 thinking 面板则开新面板
/// （boundary aggregation：相邻 thinking chunk 共享一个面板；任何非 thinking
/// 事件结束当前 burst）。
fn append_thinking_delta(body: &mut Vec<CardElement>, delta: &str) {
    if let Some(CardElement::CollapsiblePanel(panel)) = body.last_mut()
        && panel.header.title.content.contains("💭")
    {
        // 扩展尾部 thinking 面板：往末位 Markdown 追加换行 + delta。
        append_to_thinking_panel(panel, delta);
        return;
    }
    body.push(CardElement::CollapsiblePanel(CollapsiblePanel {
        expanded: false,
        header: thinking_panel_header(),
        elements: vec![CardElement::Markdown {
            content: delta.to_string(),
        }],
    }));
}

/// 在已存在的 thinking 面板里追加 delta：末位是 Markdown 就接一行，否则新建 Markdown。
fn append_to_thinking_panel(panel: &mut CollapsiblePanel, delta: &str) {
    match panel.elements.last_mut() {
        Some(CardElement::Markdown { content }) => {
            content.push('\n');
            content.push_str(delta);
        }
        _ => panel.elements.push(CardElement::Markdown {
            content: delta.to_string(),
        }),
    }
}

/// 代码级硬上限：单次 tool result 在卡片里最多展示的字符数。
/// 软上限（max_tool_output_chars）只决定“多长才折叠”，硬上限不管配置如何
/// 都生效，防止异常输出把整张卡片撑爆。
const TOOL_RESULT_HARD_LIMIT_CHARS: usize = 10240;

/// 按字符数截取（UTF-8 安全），返回不超过 `limit` 字符的前缀。
fn cap_chars(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        s.to_string()
    } else {
        s.chars().take(limit).collect()
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
    body.iter().map(element_chars).sum()
}

fn element_chars(el: &CardElement) -> usize {
    match el {
        CardElement::Markdown { content } => content.chars().count(),
        CardElement::Div { text } => text.content.chars().count(),
        CardElement::CollapsiblePanel(panel) => panel.elements.iter().map(element_chars).sum(),
        _ => 0,
    }
}
