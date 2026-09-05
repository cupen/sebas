//! 中立 UI 卡构建器（decouple-feishu-channel task 3）。
//!
//! 原来由 feishu crate 提供的「一次性 UI 卡」（帮助 / 权限 /
//! 状态 / 错误）现在由 router 直接用中立 `ChannelCard` + `ChannelElement`
//! 构建；feishu adapter 的 `render_standalone_card` 负责把中立卡渲染成飞书
//! card JSON。内容/布局与迁移前逐一对应（42 行柱状组、折叠面板收纳长参数、
//! 错误卡代码围栏等），保证既有卡面文案与交互不变。

use sebas_channels::card::{
    Behavior, ChannelCard, ChannelElement, CollapsiblePanel, DivText, Field, RichText,
};
use serde_json::Value;
use std::collections::HashMap;

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

/// 权限卡参数展示的代码级硬上限：完整参数（含折叠面板里的 JSON）超过即截断，
/// 防止整卡超过飞书卡片消息体 30KB 上限导致发卡失败、权限流程卡死。
const PERMISSION_ARGS_HARD_LIMIT_CHARS: usize = 8192;

/// 单行字段值的预览长度；长文本（Write content、Edit old/new）只显示开头，
/// 完整内容由折叠面板收纳。
const PERMISSION_FIELD_PREVIEW_CHARS: usize = 300;

/// Bash 命令摘要行：短命令用行内代码（`$ cmd`），多行/超长用 bash 代码块。
fn command_headline(cmd: &str) -> ChannelElement {
    let capped = preview_chars(cmd, PERMISSION_ARGS_HARD_LIMIT_CHARS);
    let content = if capped.contains('\n') || capped.chars().count() > 160 {
        format!("```bash\n{capped}\n```")
    } else {
        format!("`$ {capped}`")
    };
    ChannelElement::Markdown { content }
}

/// 把 args 里没被摘要行消费的 key 渲染成 div.fields 行（B 方案）。
/// `skip` 里的 key 已出现在摘要行，不再重复。返回 None 表示没有可展示字段。
fn args_field_rows(args: &Value, skip: &[&str]) -> Option<ChannelElement> {
    let map = args.as_object()?;
    let fields: Vec<Field> = map
        .iter()
        .filter(|(k, _)| !skip.contains(&k.as_str()))
        .map(|(k, v)| Field {
            is_short: false,
            text: RichText {
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
    Some(ChannelElement::Fields(fields))
}

/// 折叠面板：收纳完整参数 JSON（默认收起），受硬上限保护，截断时附灰注。
fn full_args_panel(args: &Value) -> ChannelElement {
    let pretty = serde_json::to_string_pretty(args).unwrap_or_default();
    let capped = preview_chars(&pretty, PERMISSION_ARGS_HARD_LIMIT_CHARS);
    let mut elements = vec![ChannelElement::Markdown {
        content: format!("```json\n{capped}\n```"),
    }];
    if capped.chars().count() < pretty.chars().count() {
        elements.push(note_element("（参数过长，已截断）".into()));
    }
    ChannelElement::CollapsiblePanel(CollapsiblePanel {
        expanded: false,
        header_title: RichText::plain("完整参数"),
        icon_token: "down-small-ccm_outlined".into(),
        elements,
    })
}

/// 灰注 Div 元素（notation size + grey）。
fn note_element(content: String) -> ChannelElement {
    ChannelElement::Div {
        text: DivText {
            tag: "plain_text".into(),
            content,
            text_size: Some("notation".into()),
            text_color: Some("grey".into()),
        },
    }
}

/// 把工具调用参数渲染成易读元素序列（方案 A+B）：
///
/// - Bash 命令独立成摘要行（`$ ...`），其余参数 field 行；
/// - 文件/链接类工具路径或 URL 摘要行 + 其余 field 行；
/// - 长文本字段行内预览，完整参数收进折叠面板；
/// - 未知/嵌套参数回退 pretty JSON 代码块（原行为）。
///
/// 所有输出受硬上限保护，整卡不会超飞书 30KB。
fn permission_args_elements(tool_name: &str, args: &Value) -> Vec<ChannelElement> {
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
            els.push(ChannelElement::Markdown {
                content: format!("📄 `{}`", preview_chars(p, PERMISSION_FIELD_PREVIEW_CHARS)),
            });
            if let Some(rows) = args_field_rows(args, &["file_path", "path"]) {
                els.push(rows);
            }
        } else if let Some(u) = args.get("url").and_then(Value::as_str) {
            els.push(ChannelElement::Markdown {
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
    vec![ChannelElement::Markdown {
        content: format!("```json\n{capped}\n```"),
    }]
}

/// 权限卡：`{tool} 想要执行：` + 参数展示 + 灰注 + 三个放行/拒绝按钮。
pub fn permission_card(session_id: &str, request_id: &str, tool_name: &str, args: &Value) -> ChannelCard {
    let mut card = ChannelCard::new("⚠ 权限请求", "orange");
    card.elements.push(ChannelElement::Markdown {
        content: format!("**{tool_name}** 想要执行："),
    });
    for el in permission_args_elements(tool_name, args) {
        card.elements.push(el);
    }
    card.elements.push(note_element(
        "本会话不再询问 = 之后本会话所有权限请求自动放行；/new 或会话结束后失效".into(),
    ));
    let btn = |label: &str, style: &str, decision: &str| ChannelElement::Button {
        text: RichText::plain(label),
        style: style.into(),
        behaviors: vec![Behavior {
            r#type: "callback".into(),
            value: serde_json::json!({
                "session_id": session_id,
                "request_id": request_id,
                "decision": decision,
            }),
        }],
    };
    card.elements.push(btn("本次允许", "primary", "allow_once"));
    card.elements.push(btn("本会话不再询问", "default", "allow_session"));
    card.elements.push(btn("拒绝", "danger", "deny"));
    card
}

/// 按钮回调引用了一个已不存在的会话时展示（进程退出 / 重启后）。
pub fn dead_session_card() -> ChannelCard {
    let mut card = ChannelCard::new("会话已结束", "grey");
    card.elements.push(ChannelElement::Markdown {
        content: "会话已结束，请发送 /new 开启新的会话。".into(),
    });
    card
}

/// 恢复映射的会话在 claude 侧已不存在（resume 被拒、透明回退新会话）。
pub fn session_lost_card() -> ChannelCard {
    let mut card = ChannelCard::new("已开启新会话", "orange");
    card.elements.push(ChannelElement::Markdown {
        content: "⚠️ 未找到原会话记录（可能已被清理），本次对话将作为新会话继续。".into(),
    });
    card
}

/// 权限卡点击后的原位替换：`label` 是 "✅ 已允许（仅此一次）" 等，无按钮。
pub fn resolved_permission_card(label: &str) -> ChannelCard {
    let mut card = ChannelCard::new("权限请求", "blue");
    card.elements.push(ChannelElement::Markdown {
        content: label.into(),
    });
    card
}

/// 过期权限点击（请求早已处理/从未跟踪）时的提示卡。
pub fn expired_permission_card() -> ChannelCard {
    let mut card = ChannelCard::new("权限请求", "grey");
    card.elements.push(ChannelElement::Markdown {
        content: "⚠ 请求已过期，该工具调用已被处理或取消。".into(),
    });
    card
}

/// 启动失败/超时卡：红卡，多行或超长消息用代码围栏包裹。
pub fn error_card(message: &str) -> ChannelCard {
    let mut card = ChannelCard::new("❌ 启动失败", "red");
    if message.contains('\n') || message.len() > 120 {
        card.elements.push(ChannelElement::Markdown {
            content: format!("```\n{message}\n```"),
        });
    } else {
        card.elements.push(ChannelElement::Markdown {
            content: message.to_string(),
        });
    }
    card
}

// ── 帮助卡（/help） ─────────────────────────────────────────────────────────

/// Command groups for the interactive help card.
/// Maps group key → (tab label, list of (command, description)).
type HelpGroupTable = HashMap<&'static str, (&'static str, Vec<(&'static str, &'static str)>)>;

fn help_command_groups() -> HelpGroupTable {
    let mut m: HelpGroupTable = HashMap::new();
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
                ("/router on|off|restart|status", "管理 router"),
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

/// 交互式帮助卡：tab 按钮行 + 当前组的命令按钮（column_set 横排，
/// 超长命令独立整行）。`group` 非法时回退 "session"。
pub fn help_card(group: &str, theme: &str) -> ChannelCard {
    let groups = help_command_groups();
    let group = if groups.contains_key(group) {
        group
    } else {
        "session"
    };
    let (tab_label, commands) = &groups[group];

    let mut card = ChannelCard::new(format!("📖 帮助 — {tab_label}"), theme);

    // Tab buttons row: all groups as buttons, current group highlighted.
    let tab_order = ["session", "system", "service", "other"];
    let mut tab_buttons = Vec::new();
    for key in tab_order.iter() {
        let (label, _) = &groups[key];
        let style = if *key == group { "primary" } else { "default" };
        tab_buttons.push(ChannelElement::Button {
            text: RichText::plain(*label),
            style: style.into(),
            behaviors: vec![Behavior {
                r#type: "callback".into(),
                value: serde_json::json!({"help_tab": key}),
            }],
        });
    }
    card.elements.extend(tab_buttons);

    // Divider + command buttons for the selected group.
    // 横排：每行 2-3 个 column，每个 column 内 = button(cmd) + 灰色 Div(desc)
    // 垂直堆叠。超长 cmd（如 "/router on|off|restart|status"）单独占满整行。
    card.elements.push(ChannelElement::Hr);
    let mut pending_columns: Vec<sebas_channels::card::Column> = Vec::new();
    let mut current_width = 0_u8; // 当前行累计「视觉权重」；满 6 触发 flush
    let wide_weight: u8 = 6;
    let normal_weight: u8 = 2;
    let max_row_weight: u8 = 6;
    for (cmd, desc) in commands {
        // 视觉权重估算：cmd 含空格/竖线或长度 >14 → 超长，独立成行；
        // 否则按正常权重 2 计算（行满 3 个时 flush）。
        let cmd_len = cmd.chars().count();
        let is_wide = cmd.contains(' ') || cmd.contains('|') || cmd_len > 14;
        let weight = if is_wide { wide_weight } else { normal_weight };
        // 触发 flush：当前行已有内容 + 加这一格会超 6。
        if !pending_columns.is_empty() && current_width + weight > max_row_weight {
            card.elements.push(ChannelElement::ColumnSet {
                flex_mode: false,
                horizontal_spacing: Some("8px".into()),
                columns: std::mem::take(&mut pending_columns),
            });
            current_width = 0;
        }
        // 单列：button 在上、灰色 desc Div 在下；按钮文本只保留 cmd（desc
        // 单独渲染避免长文本溢出列宽）。
        let button = ChannelElement::Button {
            text: RichText::plain(*cmd),
            style: "default".into(),
            behaviors: vec![Behavior {
                r#type: "callback".into(),
                value: serde_json::json!({"help_cmd": cmd}),
            }],
        };
        let desc_div = ChannelElement::Div {
            text: DivText {
                tag: "plain_text".into(),
                content: (*desc).into(),
                text_size: Some("notation".into()),
                text_color: Some("grey".into()),
            },
        };
        pending_columns.push(sebas_channels::card::Column {
            width: None,
            elements: vec![button, desc_div],
            vertical_spacing: Some("4px".into()),
            horizontal_align: Some("center".into()),
        });
        current_width += weight;
    }
    if !pending_columns.is_empty() {
        card.elements.push(ChannelElement::ColumnSet {
            flex_mode: false,
            horizontal_spacing: Some("8px".into()),
            columns: pending_columns,
        });
    }

    card.elements.push(note_element(
        "提示：点击分组 tab 切换，点击命令直接执行".into(),
    ));
    card
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_card_uses_column_set_rows() {
        for group in ["session", "system", "service", "other"] {
            let card = help_card(group, "blue");
            let colset_rows: Vec<&ChannelElement> = card
                .elements
                .iter()
                .filter(|e| matches!(e, ChannelElement::ColumnSet { .. }))
                .collect();
            assert!(
                !colset_rows.is_empty(),
                "group={group}: help card must contain ≥1 column_set row"
            );
            for row in &colset_rows {
                let ChannelElement::ColumnSet { columns, .. } = row else {
                    continue;
                };
                assert!(
                    (1..=3).contains(&columns.len()),
                    "group={group}: column_set row must have 1-3 columns, got {}",
                    columns.len()
                );
                for col in columns {
                    assert_eq!(
                        col.elements.len(),
                        2,
                        "group={group}: each column must hold exactly 2 stacked items (button + desc)"
                    );
                    assert!(
                        matches!(col.elements[0], ChannelElement::Button { .. }),
                        "group={group}: column[0] must be a Button"
                    );
                    assert!(
                        matches!(col.elements[1], ChannelElement::Div { .. }),
                        "group={group}: column[1] must be a Div (description)"
                    );
                }
            }
        }
    }

    #[test]
    fn help_card_wide_command_takes_full_row() {
        let card = help_card("service", "blue");
        let mut found_wide = false;
        for el in &card.elements {
            if let ChannelElement::ColumnSet { columns, .. } = el
                && columns.len() == 1
                && let ChannelElement::Button { text, .. } = &columns[0].elements[0]
                && text.content.contains("/router")
            {
                found_wide = true;
                break;
            }
        }
        assert!(
            found_wide,
            "/router command should be rendered as a single-column row (full-width)"
        );
    }

    #[test]
    fn permission_card_has_three_v2_buttons() {
        let card = permission_card("s1", "r1", "Bash", &serde_json::json!({"cmd": "ls"}));
        let buttons: Vec<&ChannelElement> = card
            .elements
            .iter()
            .filter(|e| matches!(e, ChannelElement::Button { .. }))
            .collect();
        assert_eq!(buttons.len(), 3, "3 first-class V2 button elements");
        // —— 序列化后应是 v2 button + behaviors，无 V1 action —— 由 adapter
        // 测试覆盖 wire 形状；这里断言中立模型层面无 action 容器概念。
    }

    #[test]
    fn error_card_red_with_fenced_long_message() {
        let card = error_card(&"x".repeat(121));
        assert_eq!(card.title, "❌ 启动失败");
        assert_eq!(card.theme, "red");
        assert!(
            serde_json::to_string(&card.elements).unwrap().contains("```"),
            "long message must be fenced"
        );
    }

    #[test]
    fn dead_session_and_expired_cards_have_expected_copy() {
        let dead = dead_session_card();
        assert_eq!(dead.title, "会话已结束");
        let expired = expired_permission_card();
        assert_eq!(expired.theme, "grey");
        let resolved = resolved_permission_card("✅ 已允许（仅此一次）");
        assert!(serde_json::to_string(&resolved.elements)
            .unwrap()
            .contains("已允许（仅此一次）"));
    }
}