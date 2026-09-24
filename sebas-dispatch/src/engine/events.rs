//! Session event stream observers 的投影逻辑（openspec/changes/add-core-session-channel）。
//!
//! 本模块原本承载 `SessionInfo` / `SessionEvent` / `TurnEntry` 等线格式类型；
//! 它们已迁往中立共享域层 `sebas_domain::session`（add-domain-layer 3.1，
//! design D3 原位再导出），这里保留**引擎侧的投影/聚合逻辑**
//! （`count_chat_messages`、折叠标题、failure_class 词表）与既有路径。
//!
//! 会话由扁平化的 [`sebas_channels::ChannelKey`] 寻址：`channel` 是来源
//! 渠道名，`key` 是 core 不解释的渠道中立 reference。

// 中立契约类型已迁往 `sebas_domain::session`（add-domain-layer 3.1，design
// D3 原位再导出）：`sebas_dispatch::engine::events::*` 乃至
// `sebas_dispatch::*` 的既有路径全部保持可解析。
pub use sebas_domain::session::{
    PendingApproval, RemoteSessionView, SessionEvent, SessionInfo, TurnElementType, TurnEntry,
    TurnKind, TurnStreamEvent,
};

/// 可见回复段的计数口径（rail-declutter-unread D2，用户拍板「可见回复段」）：
///
/// - 只数 `kind = "content"` 且 `element_type ∈ {markdown, error}` 的条目；
///   prompt、thinking、tool 一律不计；
/// - **相邻连续的 markdown 条目合并为一段**（ACP 路径逐 delta 落账，一段
///   流式回复 = 一串 delta 条目；与前端 transcript 把连续 markdown 拼进同
///   一个文本块同粒度），被 prompt / thinking / tool / error 打断则另起一段；
/// - 每条 `error` 条目独立计 1（前端逐条渲染错误气泡）；
/// - 空内容条目跳过、**不断段**（前端 `groupConversation` 同规则）。
///
/// 注意与 seen-boundary seam 的差异：seam 按**轮**分界，徽标按**段**计数
/// ——两者共用同一游标（D3）但聚合粒度不同，属有意为之，测试分别钉住。
pub fn count_chat_messages(entries: &[TurnEntry]) -> u64 {
    let mut count = 0u64;
    let mut in_markdown_run = false;
    for e in entries {
        if e.content.is_empty() {
            // 空条目对前端不可见：不计数、不打断当前段。
            continue;
        }
        if e.kind != TurnKind::Content {
            in_markdown_run = false;
            continue;
        }
        // （type-session-vocabularies 4.2）类型化 match：新增一个元素类型必须
        // 在这里显式表态（编译失败），不再靠字符串落到 `_` 兜底。
        match e.element_type {
            TurnElementType::Markdown => {
                if !in_markdown_run {
                    count += 1;
                    in_markdown_run = true;
                }
            }
            TurnElementType::Error => {
                count += 1;
                in_markdown_run = false;
            }
            // 其余元素类型都不构成「可见回复段」并打断当前段（既有 `_` 兜底
            // 的逐字展开）：thinking / tool 是正文附属，notice 与
            // permission_mode_result 是中性提示，未知取值照同样口径处理。
            TurnElementType::Thinking
            | TurnElementType::Tool
            | TurnElementType::Notice
            | TurnElementType::PermissionModeResult
            | TurnElementType::Unknown(_) => in_markdown_run = false,
        }
    }
    count
}

/// 错误条目的失败分类词表（design D5）：spawn = spawn 失败；stall = 回合
/// 停滞强收；generic = 回合终态错误（含 refusal）。前端按词表映射标签。
pub mod failure_class {
    pub const SPAWN: &str = "spawn";
    pub const STALL: &str = "stall";
    pub const GENERIC: &str = "generic";
}

/// 折叠标题的硬上限（workbench-agent-identity-and-process-folds 1.2）：
/// 工具名 + 参数摘要整体 200 字符，防异常超长参数撑爆折叠栏。
const TITLE_MAX_CHARS: usize = 200;

/// 关键参数的偏好键序（design D2）：路径类 > 模式 > URL > 命令 > 查询，
/// 覆盖 read/edit/write、glob/grep、web fetch/search、bash 等主流工具的
/// 实际参数形态。
const KEY_ARG_PREFERRED: [&str; 11] = [
    "path",
    "file_path",
    "absolute_path",
    "file",
    "dir",
    "directory",
    "cwd",
    "pattern",
    "url",
    "command",
    "query",
];

/// 从工具 args JSON 提取关键参数摘要（workbench-agent-identity-and-process-folds
/// 1.2）：偏好键序取第一个**非空**字符串值；全未命中退化为对象里第一个
/// 字符串值；再没有（非对象 / 无字符串值）返回 `None`，标题退化为纯工具名。
fn key_arg_from_args(args: &serde_json::Value) -> Option<String> {
    let obj = args.as_object()?;
    for key in KEY_ARG_PREFERRED {
        if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    obj.values()
        .find_map(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 按**字符数**截断（非字节）：中文等多字节字符不切出半个 UTF-8 序列。
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// （workbench-agent-identity-and-process-folds 1.2）工具条目的结构化标题：
/// `done=false` 为调用开始（`{tool} · {key_arg}`），`done=true` 带完成前缀
/// （`✓ {tool} · {key_arg}`）；提取不到关键参数就只有工具名（带/不带 ✓）。
/// 注意：`AcpEvent::ToolEnd` 不携带 args（wire 无 call id 可配对），完成态
/// 调用方传 `None`，标题即 `✓ {tool}`。
pub(crate) fn tool_entry_title(
    done: bool,
    tool_name: &str,
    args: Option<&serde_json::Value>,
) -> String {
    let mut title = match args.and_then(key_arg_from_args) {
        Some(arg) => format!("{tool_name} · {arg}"),
        None => tool_name.to_string(),
    };
    if done {
        title.insert_str(0, "✓ ");
    }
    truncate_chars(&title, TITLE_MAX_CHARS)
}

/// Wall-clock seconds since the UNIX epoch：唯一实现在
/// `sebas_domain::prim::now_unix`（add-domain-layer 2.5）；u64 形态在
/// 唯一调用点内联换算，不留第二份定义。

#[cfg(test)]
mod tests {
    use super::*;
    use sebas_channels::ChannelKey;

    #[test]
    fn session_event_round_trips_through_serde() {
        // 1.1 验收：每个变体经 serde 往返后与原值一致。
        let info = SessionInfo {
            channel: "feishu".into(),
            key: "oc_1\0om_t".into(),
            session_id: Some("s1".into()),
            status: "active".into(),
            phase: Some("DONE".into()),
            user_prompt: Some("hello".into()),
            last_active_unix: 1234,
            project_dir: Some("/tmp/p".into()),
            current_model: Some("m1".into()),
            available_models: Some(vec!["m1".into(), "m2".into()]),
            agent_kind: Some("claude".into()),
            usage: None,
            // wire-webui-sebas-agent-e2e D4：执行体标签随快照/事件往返。
            backend: Some("native".into()),
            // workbench-turn-queue D6：pending 视图随 SessionInfo 往返。
            pending: vec![crate::state::PendingSubmission {
                id: 7,
                text: "queued behind the running turn".into(),
                position: 0,
                disposition: crate::state::PendingDisposition::Turn,
                priority: false,
            }],
            remote: None,
            desired_mode: crate::engine::ask_mode(),
            effective_mode: None,
            // rail-declutter-unread：msg_count 随 SessionInfo 往返。
            msg_count: 0,
            // fix-pending-queue-liveness 2.3：turn_engaged 随 SessionInfo 往返。
            turn_engaged: true,
            spawn_failure_reason: None,
            parked_approvals: 0,
            label: None,
            // session-slash-commands：命令表随 SessionInfo 往返。
            available_commands: vec![
                sebas_acp::AvailableCommand {
                    name: "goal".into(),
                    description: "Set a goal".into(),
                    hint: Some("<condition>".into()),
                },
                sebas_acp::AvailableCommand {
                    name: "compact".into(),
                    description: "Clear context".into(),
                    hint: None,
                },
            ],
        };
        let cases = vec![
            SessionEvent::Created {
                session: info.clone(),
            },
            SessionEvent::Updated { session: info },
            SessionEvent::Removed {
                channel: "feishu".into(),
                key: "oc_2".into(),
            },
            // workbench-turn-queue 5.2：丢弃标注随事件往返。
            SessionEvent::PendingDropped {
                channel: "web".into(),
                key: "web-1".into(),
                dropped: vec![crate::state::PendingSubmission {
                    id: 3,
                    text: "never ran".into(),
                    position: 0,
                    disposition: crate::state::PendingDisposition::Turn,
                    priority: false,
                }],
            },
            SessionEvent::Resync,
        ];
        for ev in cases {
            let json = serde_json::to_string(&ev).expect("serialize");
            let back: SessionEvent = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, ev, "round-trip mismatch for {json}");
        }
    }

    #[test]
    fn session_event_uses_type_tag() {
        // wire 形态带 "type" tag，与 control RPC 的 cmd tag 姿态一致。
        let ev = SessionEvent::Removed {
            channel: "feishu".into(),
            key: "oc_x".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "removed");
        assert_eq!(json["channel"], "feishu");
        assert_eq!(json["key"], "oc_x");
    }

    #[test]
    fn channel_key_round_trips_through_flattened_info() {
        let k = ChannelKey::feishu("oc_x", Some("t1"));
        let info = SessionInfo {
            channel: k.channel_str().to_string(),
            key: k.reference.clone(),
            session_id: Some("s1".into()),
            status: "active".into(),
            phase: None,
            user_prompt: None,
            last_active_unix: 0,
            project_dir: None,
            current_model: None,
            available_models: None,
            agent_kind: None,
            usage: None,
            backend: None,
            pending: Vec::new(),
            remote: None,
            desired_mode: crate::engine::ask_mode(),
            effective_mode: None,
            msg_count: 0,
            // fix-pending-queue-liveness 2.3：turn_engaged 缺省兼容。
            turn_engaged: false,
            spawn_failure_reason: None,
            parked_approvals: 0,
            label: None,
            // session-slash-commands：无发现能力会话的命令表恒空。
            available_commands: Vec::new(),
        };
        assert_eq!(info.channel, "feishu");
        assert_eq!(info.key, "oc_x\0t1");
        assert_eq!(info.channel_key(), k);
    }

    #[test]
    fn turn_entry_round_trips_through_serde() {
        let e = TurnEntry::prompt(3, "fix the bug");
        let back: TurnEntry = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    /// workbench-agent-identity-and-process-folds 1.1：旧持久化 JSON 无
    /// `title` 字段仍反序列化（→ None，不报错）；带 title 的条目完整往返；
    /// None 序列化时**省略键**（wire 只在有值时多一个可选键）。
    #[test]
    fn turn_entry_title_field_is_additive() {
        // 旧形状：无 title 字段。
        let legacy = r#"{
            "position": 4,
            "kind": "content",
            "element_type": "tool",
            "content": "📖 **Read**",
            "created_at_unix": 42
        }"#;
        let back: TurnEntry = serde_json::from_str(legacy).unwrap();
        assert_eq!(back.title, None);
        assert_eq!(back.position, 4);
        assert_eq!(back.element_type, sebas_domain::session::TurnElementType::Tool);

        // 带 title：完整往返。
        let titled = TurnEntry::tool(0, "📖 **Read**").with_title(tool_entry_title(
            false,
            "Read",
            Some(&serde_json::json!({"file_path": "src/main.rs"})),
        ));
        let json = serde_json::to_string(&titled).unwrap();
        assert_eq!(serde_json::from_str::<TurnEntry>(&json).unwrap(), titled);
        assert!(json.contains(r#""title""#), "{json}");

        // None：键不上 wire。
        let untitled = TurnEntry::tool(1, "✓ **Read**");
        let json = serde_json::to_string(&untitled).unwrap();
        assert!(!json.contains("title"), "{json}");
    }

    /// workbench-agent-identity-and-process-folds 1.2：路径类键命中——
    /// `file_path` 提取为关键参数；同现 `path` 与 `command` 时路径键优先。
    #[test]
    fn tool_title_prefers_path_keys() {
        let args = serde_json::json!({"file_path": "src/main.rs"});
        assert_eq!(
            tool_entry_title(false, "Read", Some(&args)),
            "Read · src/main.rs"
        );
        // 路径组在偏好键序里先于 command/pattern。
        let args = serde_json::json!({"command": "cat f", "path": "src/"});
        assert_eq!(tool_entry_title(false, "Read", Some(&args)), "Read · src/");
    }

    /// workbench-agent-identity-and-process-folds 1.2：无路径键时按偏好键序
    /// 回退到 command / pattern。
    #[test]
    fn tool_title_falls_back_to_command_and_pattern() {
        let args = serde_json::json!({"command": "cargo test -p sebas-dispatch"});
        assert_eq!(
            tool_entry_title(false, "Bash", Some(&args)),
            "Bash · cargo test -p sebas-dispatch"
        );
        let args = serde_json::json!({"pattern": "TurnEntry", "path": ""});
        assert_eq!(
            tool_entry_title(false, "Grep", Some(&args)),
            "Grep · TurnEntry",
            "空字符串的偏好键视为未命中，继续向后找"
        );
    }

    /// workbench-agent-identity-and-process-folds 1.2：没有任何字符串参数
    /// （空对象 / 非字符串值 / 非对象 args / ToolEnd 无 args）时标题退化为
    /// 纯工具名；完成态带 `✓ ` 前缀。其余字符串值兜底（偏好键全未命中时
    /// 取对象里第一个字符串值）。
    #[test]
    fn tool_title_without_string_args_is_tool_name_only() {
        assert_eq!(
            tool_entry_title(false, "Bash", Some(&serde_json::json!({}))),
            "Bash"
        );
        assert_eq!(
            tool_entry_title(false, "Bash", Some(&serde_json::json!({"count": 3}))),
            "Bash"
        );
        // 非对象 args（null / 数组）同样只有工具名。
        assert_eq!(
            tool_entry_title(false, "Bash", Some(&serde_json::Value::Null)),
            "Bash"
        );
        assert_eq!(
            tool_entry_title(false, "Bash", Some(&serde_json::json!(["a", "b"]))),
            "Bash"
        );
        // ToolEnd：wire 无 args → 传 None，只有 `✓ {tool}`。
        assert_eq!(tool_entry_title(true, "Bash", None), "✓ Bash");
        assert_eq!(tool_entry_title(false, "Bash", None), "Bash");
        // 偏好键全未命中：第一个字符串值兜底。
        assert_eq!(
            tool_entry_title(false, "Tool", Some(&serde_json::json!({"x": "y", "n": 1}))),
            "Tool · y"
        );
    }

    /// workbench-agent-identity-and-process-folds 1.2：标题整体 200 字符上限，
    /// 按字符截断（多字节中文不切半个、不 panic）。
    #[test]
    fn tool_title_caps_at_200_chars_on_char_boundaries() {
        // 300 个中文字符的 command：截到 200 字符。
        let long = "读".repeat(300);
        let args = serde_json::json!({ "command": long });
        let title = tool_entry_title(false, "Bash", Some(&args));
        assert_eq!(title.chars().count(), 200);
        assert!(title.starts_with("Bash · "));
        // 恰好 200 字符（"Bash · " 占 7 字符 + 193 字符参数）：不截断。
        let exact = "a".repeat(193);
        let title = tool_entry_title(
            false,
            "Bash",
            Some(&serde_json::json!({ "command": exact })),
        );
        assert_eq!(title.chars().count(), 200);
        assert!(title.ends_with(&exact));
    }
}

/// （extract-im-service 2.3）usage 字段 serde 兼容：旧形状（无 usage）仍反
/// 序列化；带 usage 的形状完整往返。
#[test]
fn session_info_usage_field_is_additive() {
    use sebas_channels::card::AppUsage;

    let full = SessionInfo {
        channel: "feishu".into(),
        key: "oc_u".into(),
        session_id: Some("s1".into()),
        status: "active".into(),
        phase: Some("OnIt".into()),
        user_prompt: Some("p".into()),
        last_active_unix: 1,
        project_dir: None,
        current_model: None,
        available_models: None,
        agent_kind: None,
        backend: None,
        pending: Vec::new(),
        remote: None,
        usage: Some(AppUsage {
            model: Some("claude-x".into()),
            total_input: 10,
            total_output: 25,
        }),
        desired_mode: crate::engine::ask_mode(),
        effective_mode: None,
        msg_count: 3,
        // fix-pending-queue-liveness 2.3：usage 段用例顺带覆盖 turn_engaged。
        turn_engaged: false,
        spawn_failure_reason: None,
        parked_approvals: 0,
            label: None,
        available_commands: Vec::new(),
    };
    let json = serde_json::to_string(&full).unwrap();
    let back: SessionInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(back, full);

    // 旧形状：无 usage 字段的 JSON 反序列化为 None。
    let legacy = r#"{"channel":"feishu","key":"oc_u","session_id":"s1","status":"active","phase":null,"user_prompt":null,"last_active_unix":1,"project_dir":null,"current_model":null,"available_models":null,"agent_kind":null}"#;
    let back: SessionInfo = serde_json::from_str(legacy).unwrap();
    assert_eq!(back.usage, None);
    // workbench-turn-queue：无 pending 字段同样可读（默认空栈）。
    assert_eq!(back.pending, Vec::new());
    // rail-declutter-unread：无 msg_count 字段的旧报文反序列化为 0。
    assert_eq!(back.msg_count, 0);
}

/// session-slash-commands 2.1/2.2：`available_commands` 字段的 serde 兼容。
/// 旧 core + 新前端组合：旧报文（无该键）反序列化为空表；新 core + 旧前端
/// 组合：空表时键不上 wire（wire 形状与旧版本一致），非空表完整往返。
#[test]
fn session_info_available_commands_field_is_additive() {
    // 新形状（非空表）：完整往返。
    let full = SessionInfo {
        channel: "web".into(),
        key: "slash-1".into(),
        session_id: Some("s1".into()),
        status: "active".into(),
        phase: None,
        user_prompt: None,
        last_active_unix: 1,
        project_dir: None,
        current_model: None,
        available_models: None,
        agent_kind: None,
        usage: None,
        backend: None,
        pending: Vec::new(),
        remote: None,
        desired_mode: crate::engine::ask_mode(),
        effective_mode: None,
        msg_count: 0,
        // fix-pending-queue-liveness 2.3：turn_engaged 随 SessionInfo 往返。
        turn_engaged: true,
        spawn_failure_reason: None,
        parked_approvals: 0,
            label: None,
        available_commands: vec![sebas_acp::AvailableCommand {
            name: "goal".into(),
            description: "Set a goal".into(),
            hint: Some("<condition>".into()),
        }],
    };
    let json = serde_json::to_string(&full).unwrap();
    let back: SessionInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(back, full);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["available_commands"][0]["name"], "goal");
    assert_eq!(value["available_commands"][0]["hint"], "<condition>");

    // 新 core + 旧前端：空表 → 键省略（旧消费端看到的 wire 无新键）。
    let mut empty = full.clone();
    empty.available_commands = Vec::new();
    let json = serde_json::to_string(&empty).unwrap();
    assert!(
        !json.contains("available_commands"),
        "empty table must be omitted from the wire: {json}"
    );

    // 旧 core + 新前端：旧报文（无 available_commands 键）→ 空表，不报错。
    let legacy = r#"{"channel":"web","key":"slash-2","session_id":"s1","status":"active","phase":null,"user_prompt":null,"last_active_unix":1,"project_dir":null,"current_model":null,"available_models":null,"agent_kind":null}"#;
    let back: SessionInfo = serde_json::from_str(legacy).unwrap();
    assert!(
        back.available_commands.is_empty(),
        "missing key must deserialize to an empty (no command surface) table"
    );
}

/// workbench-turn-queue 5.2：PendingDropped 携带被丢弃条目（id + 文本），
/// wire 形状带 snake_case 的 type 标签。
#[test]
fn pending_dropped_event_carries_the_dropped_entries() {
    let ev = SessionEvent::PendingDropped {
        channel: "web".into(),
        key: "web-drop".into(),
        dropped: vec![
            crate::state::PendingSubmission {
                id: 1,
                text: "one".into(),
                position: 0,
                disposition: crate::state::PendingDisposition::Staging,
                priority: false,
            },
            crate::state::PendingSubmission {
                id: 2,
                text: "two".into(),
                position: 1,
                disposition: crate::state::PendingDisposition::Turn,
                priority: true,
            },
        ],
    };
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["type"], "pending_dropped");
    assert_eq!(json["dropped"][1]["text"], "two");
    assert_eq!(json["dropped"][1]["disposition"], "turn");
    assert_eq!(json["dropped"][1]["priority"], true);
    let back: SessionEvent = serde_json::from_value(json).unwrap();
    assert_eq!(back, ev);
}

/// rail-declutter-unread D2（计数口径）：流式 delta 合并成段——一串相邻
/// markdown 计 1，被 tool/thinking 打断后另起一段，error 逐条各计 1。
#[test]
fn chat_message_count_merges_adjacent_markdown_deltas() {
    let log = vec![
        TurnEntry::prompt(0, "do it"),
        // 一段流式回复：3 个 delta = 1 段。
        TurnEntry::markdown(1, "let me "),
        TurnEntry::markdown(2, "check "),
        TurnEntry::markdown(3, "that."),
        // 工具噪声：不计、但打断当前段。
        TurnEntry::tool(4, "📖 **read_file**"),
        TurnEntry::tool(5, "✓ **read_file**"),
        // 工具后的新文本段：+1。
        TurnEntry::markdown(6, "done."),
        // thinking 噪声：不计、打断。
        TurnEntry::thinking(7, "hmm"),
        // 错误条目：逐条计 1。
        TurnEntry::error(8, "**spawn failed**: boom"),
        TurnEntry::error(9, "**spawn failed**: boom again"),
    ];
    // 段 1（三连 delta）+ 工具/thinking 打断后的段 2 + 两条 error 逐条 = 4。
    assert_eq!(count_chat_messages(&log), 4, "{log:?}");
}

/// rail-declutter-unread D2（计数口径）：process noise 不计数——只有
/// thinking / tool / prompt 的会话段数为 0；空条目不计数也不断段。
#[test]
fn chat_message_count_ignores_noise_and_empty_entries() {
    // 纯噪声 = 0。
    let noise = vec![
        TurnEntry::prompt(0, "hello"),
        TurnEntry::thinking(1, "thinking"),
        TurnEntry::tool(2, "📖 **bash**"),
        TurnEntry::tool(3, "✓ **bash**"),
    ];
    assert_eq!(count_chat_messages(&noise), 0);
    // 空内容条目：不计数、不打断相邻 markdown 的连续性（前端同规则）。
    let with_empty = vec![
        TurnEntry::markdown(0, "first"),
        TurnEntry {
            position: 1,
            kind: "content".into(),
            element_type: "markdown".into(),
            content: String::new(),
            created_at_unix: 1,
            title: None,
            failure_class: None,
        },
        TurnEntry::markdown(2, " still same segment"),
    ];
    assert_eq!(count_chat_messages(&with_empty), 1);
    // 空 transcript = 0。
    assert_eq!(count_chat_messages(&[]), 0);
}

/// workbench-live-conversation-flow 1.1：TurnStreamEvent serde 往返，
/// wire 形状带 channel/key/entries（与 SessionEvent::PendingDropped 同构
/// 的寻址字段）。
#[test]
fn turn_stream_event_round_trips() {
    let ev = TurnStreamEvent {
        channel: "web".into(),
        key: "web-1".into(),
        entries: vec![
            TurnEntry::markdown(3, "hello "),
            TurnEntry::markdown(4, "world"),
        ],
    };
    let json = serde_json::to_string(&ev).unwrap();
    let back: TurnStreamEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ev);
    assert!(json.contains("\"channel\":\"web\""));
    assert!(json.contains("\"entries\":["));
}

/// fix-pending-queue-liveness 2.3：`turn_engaged` 字段的 serde 兼容。
/// 旧 core 报文（无该键）→ 反序列化 false（前端回退 slug 判定）；
/// 新 core 完整往返；wire 形状带布尔键。
#[test]
fn session_info_turn_engaged_field_is_additive() {
    let legacy = r#"{"channel":"web","key":"engage-1","session_id":"s1","status":"active","phase":null,"user_prompt":null,"last_active_unix":1,"project_dir":null,"current_model":null,"available_models":null,"agent_kind":null}"#;
    let back: SessionInfo = serde_json::from_str(legacy).unwrap();
    assert!(
        !back.turn_engaged,
        "missing key must deserialize to not-engaged (legacy fallback semantics)"
    );

    let engaged = SessionInfo {
        channel: "web".into(),
        key: "engage-2".into(),
        session_id: Some("s1".into()),
        status: "active".into(),
        phase: Some("OnIt".into()),
        user_prompt: None,
        last_active_unix: 1,
        project_dir: None,
        current_model: None,
        available_models: None,
        agent_kind: None,
        usage: None,
        backend: None,
        pending: Vec::new(),
        remote: None,
        desired_mode: crate::engine::ask_mode(),
        effective_mode: None,
        msg_count: 0,
        turn_engaged: true,
        spawn_failure_reason: None,
        parked_approvals: 0,
            label: None,
        available_commands: Vec::new(),
    };
    let json = serde_json::to_string(&engaged).unwrap();
    let back: SessionInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(back, engaged);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["turn_engaged"], true);
}

/// fix-pending-queue-liveness 2.2：TurnStalled 事件携带会话寻址与释放的
/// 搁浅提交数，wire 形状带 snake_case 的 type 标签，serde 完整往返。
#[test]
fn turn_stalled_event_round_trips() {
    let ev = SessionEvent::TurnStalled {
        channel: "web".into(),
        key: "web-stall".into(),
        released: 2,
    };
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["type"], "turn_stalled");
    assert_eq!(json["channel"], "web");
    assert_eq!(json["key"], "web-stall");
    assert_eq!(json["released"], 2);
    let back: SessionEvent = serde_json::from_value(json).unwrap();
    assert_eq!(back, ev);
}
