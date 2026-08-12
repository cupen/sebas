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
/// ThinkingDelta 也收进同一个父级折叠面板，与工具调用统一收纳。
/// fold_long_output=false 时：保持内联展示（结果仍受 10240 硬上限保护）。
/// + 总量兜底（24000 字符上限 + 80 递归元素上限，Hr 连后一个一起丢）。
/// PermissionRequest 不累积（走独立 SendCard）。
pub fn apply_event_to_card(body: &mut Vec<CardElement>, event: &AcpEvent, cfg: &CardConfig) {
    match event {
        AcpEvent::TextDelta { delta, .. } => {
            push_text_truncated(body, delta, cfg.max_user_text_chars, cfg.fold_long_output);
        }
        AcpEvent::ThinkingDelta { delta, .. } => {
            if cfg.thinking == ThinkingDisplay::Hide {
                // 完全丢弃：模型仍在思考，只是卡片不展示。
            } else {
                append_thinking_delta(body, delta, cfg.fold_long_output);
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
                // 所有工具面板统一收进一个父级工具面板（"🛠️ 工具"），
                // 对话流更干净。
                let tool_panel = CardElement::CollapsiblePanel(CollapsiblePanel {
                    expanded: false,
                    header: panel_header(format!("📖 {tool_name}")),
                    elements: text_with_truncation_note(
                        format!("```json\n{args_str}\n```"),
                        cfg.max_user_text_chars,
                        true,
                    ),
                });
                let parent_idx = ensure_tools_parent(body);
                if let CardElement::CollapsiblePanel(parent) = &mut body[parent_idx] {
                    parent.elements.push(tool_panel);
                }
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
                // 限制进度通知数量：超过上限时移除最旧的进度通知，
                // 防止工具面板内部元素超 100 上限。
                let progress_positions: Vec<usize> = panel
                    .elements
                    .iter()
                    .enumerate()
                    .filter(|(_, el)| matches!(el, CardElement::Div { text } if text.content.starts_with("⏳ ")))
                    .map(|(i, _)| i)
                    .collect();
                if progress_positions.len() >= MAX_PROGRESS_NOTES {
                    // 移除最旧的进度通知
                    panel.elements.remove(progress_positions[0]);
                }
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

/// 取 body 末尾属于 `tool_name` 的折叠面板（标题形如 `📖 Bash` / `✓ Bash`）。
/// 事件按顺序到达（ToolStart → ToolProgress* → ToolEnd），末尾即当前工具的面板。
///
/// 搜索顺序：
/// 1. 如果 body 中存在父级面板（"🤔 折腾中"），在其 elements 内搜索；
/// 2. 否则直接在 body 末尾搜索（兼容非折叠模式）。
fn last_tool_panel_mut<'a>(
    body: &'a mut [CardElement],
    tool_name: &str,
) -> Option<&'a mut CollapsiblePanel> {
    let suffix = format!(" {tool_name}");
    // 先在 body 中搜索父面板
    if let Some(idx) = find_tools_parent_index(body) {
        let parent = match &mut body[idx] {
            CardElement::CollapsiblePanel(p) => p,
            _ => return None,
        };
        return parent.elements.iter_mut().rev().find_map(|el| {
            match el {
                CardElement::CollapsiblePanel(panel) if panel.header.title.content.ends_with(&suffix) => Some(panel),
                _ => None,
            }
        });
    }
    // 无父面板：直接在 body 末尾搜索（fold_long_output=false 或单工具早期兼容）
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

/// 父级折叠面板的标题常量。所有思考面板和工具调用面板统一收进一个父面板，
/// 内层仍保持各自独立折叠。`is_tools_parent` 根据此常量判断。
const TOOLS_PARENT_TITLE: &str = "🤔 折腾中";

/// 确保 body 中存在一个父级折叠面板，返回其索引。
/// 已存在则直接返回；否则新建一个并 push 到 body 末尾。
fn ensure_tools_parent(body: &mut Vec<CardElement>) -> usize {
    // 搜索整个 body（父面板可能不在末尾，TextDelta 等事件会插在它后面）
    for (i, el) in body.iter().enumerate() {
        if let CardElement::CollapsiblePanel(panel) = el {
            if is_tools_parent(panel) {
                return i;
            }
        }
    }
    body.push(CardElement::CollapsiblePanel(CollapsiblePanel {
        expanded: false,
        header: panel_header(TOOLS_PARENT_TITLE.into()),
        elements: vec![],
    }));
    body.len() - 1
}

/// 判断一个 panel 是否是父级折叠面板。
fn is_tools_parent(panel: &CollapsiblePanel) -> bool {
    panel.header.title.content == TOOLS_PARENT_TITLE
}

/// 在 body 中查找父级折叠面板的索引。
fn find_tools_parent_index(body: &[CardElement]) -> Option<usize> {
    body.iter().position(|el| {
        matches!(el, CardElement::CollapsiblePanel(panel) if is_tools_parent(panel))
    })
}

/// ThinkingDelta 折叠面板的标准标题。所有 thinking 面板共用此常量，
/// `is_thinking_panel` 据此判断。改这里必须连测试一起改。
const THINKING_PANEL_TITLE: &str = "💭 思考";

/// 判断一个 panel 是否是 thinking 折叠面板。聚合逻辑专用 —— 字符串
/// `contains` 容易把任何标题含 💭 的 panel 都吞掉，相邻 deltas 会串到
/// 错的面板里。
fn is_thinking_panel(panel: &CollapsiblePanel) -> bool {
    panel.header.title.content == THINKING_PANEL_TITLE
}

/// ThinkingDelta 折叠面板的 header（复用 panel_header 的标准图标）。
fn thinking_panel_header() -> CollapsiblePanelHeader {
    panel_header(THINKING_PANEL_TITLE.into())
}

/// 把 ThinkingDelta 累积进尾部 thinking 面板；body 末尾不是 thinking 面板则开新面板
/// （boundary aggregation：相邻 thinking chunk 共享一个面板；任何非 thinking
/// 事件结束当前 burst）。
///
/// 当 `fold_long_output=true` 时，thinking 面板收进父级折叠面板。
fn append_thinking_delta(body: &mut Vec<CardElement>, delta: &str, fold_long_output: bool) {
    if fold_long_output {
        // 收进父面板：先确保父面板存在，再追加或新建 thinking 面板
        let parent_idx = ensure_tools_parent(body);
        if let CardElement::CollapsiblePanel(parent) = &mut body[parent_idx] {
            // 检查父面板末尾是否是 thinking 面板（聚合相邻 delta）
            if let Some(CardElement::CollapsiblePanel(panel)) = parent.elements.last_mut()
                && is_thinking_panel(panel)
            {
                append_to_thinking_panel(panel, delta);
                return;
            }
            // 新建 thinking 面板收进父面板
            parent.elements.push(CardElement::CollapsiblePanel(CollapsiblePanel {
                expanded: false,
                header: thinking_panel_header(),
                elements: vec![CardElement::Markdown {
                    content: delta.to_string(),
                }],
            }));
            return;
        }
    }
    // 不折叠或父面板创建失败：原行为（直接 push 到 body）
    if let Some(CardElement::CollapsiblePanel(panel)) = body.last_mut()
        && is_thinking_panel(panel)
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
/// 软上限（max_tool_output_chars）只决定”多长才折叠”，硬上限不管配置如何
/// 都生效，防止异常输出把整张卡片撑爆。
const TOOL_RESULT_HARD_LIMIT_CHARS: usize = 10240;

/// 飞书卡片 JSON 2.0 限制：每个容器（collapsible_panel/div 等）最多 100 个元素。
/// 根 body 和所有嵌套容器的递归元素总数超过此上限会导致卡片渲染失败。
/// 留 20 个余量，到 80 就开始丢旧。
const MAX_ELEMENTS: usize = 80;

/// 单个工具面板内最多保留的进度通知数。进度通知过多会撑爆容器的 100 元素上限。
const MAX_PROGRESS_NOTES: usize = 5;

/// 按字符数截取（UTF-8 安全），返回不超过 `limit` 字符的前缀。
fn cap_chars(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        s.to_string()
    } else {
        s.chars().take(limit).collect()
    }
}

/// 总量兜底（spec §7）：body 累积字符 > 24000 或递归元素总数 > 80
/// 时丢最旧元素；最旧是 Hr 则连后一个一起丢。CollapsiblePanel 优先从内部丢，
/// 避免整个面板被一次性丢弃。
fn enforce_total_budget(body: &mut Vec<CardElement>, _cfg: &CardConfig) {
    const TOTAL_BUDGET: usize = 24000;
    while total_chars(body) > TOTAL_BUDGET || total_elements(body) > MAX_ELEMENTS {
        if body.is_empty() {
            break;
        }
        drop_oldest_element(body);
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

/// 递归计算 body 中所有元素（含嵌套 CollapsiblePanel 内部）的总数。
fn total_elements(body: &[CardElement]) -> usize {
    body.iter().map(count_elements).sum()
}

/// 递归计算单个元素及其嵌套子元素的总数。
fn count_elements(el: &CardElement) -> usize {
    match el {
        CardElement::CollapsiblePanel(panel) => {
            1 + panel.elements.iter().map(count_elements).sum::<usize>()
        }
        _ => 1,
    }
}

/// 从 body 中丢掉最旧的 1 个元素。
/// - 如果最旧是 CollapsiblePanel，直接从其内部丢掉最旧的子元素（不递归）；
/// - 如果最旧是 Hr，连后一个一起丢（不留悬空分隔线）。
/// - 内部丢空后，再丢面板本身。
fn drop_oldest_element(body: &mut Vec<CardElement>) {
    if body.is_empty() {
        return;
    }
    // 最旧是 CollapsiblePanel → 从内部直接丢掉最旧的子元素（不递归嵌套）
    if let CardElement::CollapsiblePanel(panel) = &mut body[0] {
        if !panel.elements.is_empty() {
            // 直接移除最旧的子元素，避免递归进入子面板内部（如工具面板内的
            // Markdown），导致面板变空却不被丢弃。
            panel.elements.remove(0);
            return;
        }
        // 内部已空 → 丢面板本身
        body.remove(0);
        return;
    }
    // 普通元素：Hr 连后一个一起丢
    let drop_two = matches!(body[0], CardElement::Hr);
    body.remove(0);
    if drop_two && !body.is_empty() {
        body.remove(0);
    }
}
