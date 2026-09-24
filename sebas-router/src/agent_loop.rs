//! 确定性 agent-loop 规则（extend-test-model-scenarios 2.2 / 2.6）。
//!
//! 本模块是 **test 模型**（`crate::test_provider`）与 **fake 上游**
//! （`crate::fake_provider`）共用的事实源：线协议形状在
//! [`crate::anthropic_wire`]，**规则判断**（何时回 tool_use、回哪个 tool、
//! input 怎么来）在这里。两处实现各写一份规则必然漂移——下沉到此处后，
//! 漂移面只剩「选哪个 tool」这一处**有意的差异**（见下）。
//!
//! 规范（`fake-provider-upstream`「内置 agent-loop 确定性规则」，逐字同规范）：
//!
//! - 请求含非空 `tools` 且消息历史**无** `tool_result` → 某个 tool 的
//!   `tool_use` 块，`stop_reason=tool_use`；
//! - 消息历史**已含** `tool_result` → 终文本应答，`stop_reason=end_turn`；
//! - 无 `tools` → 纯文本应答。
//!
//! 两处唯一的规则差异是 tool 的选择：fake 上游按「只读/命令类偏好」挑
//! （[`pick_tool`]，让拨号演练自动收敛、不弹审批），test 模型的
//! `test/tool-use` / `test/full` 取**首个声明工具**（[`first_tool`]）并给它
//! **可触发审批**的确定性 input（[`deterministic_input_for_approval`]）——
//! debug test 模型的职责就是驱动工作台的权限路径，只读 tool 会被策略静默
//! 放行、权限流无法被覆盖（design D2：分岔只加不改，fake 侧语义不动）。

use serde_json::{Value, json};

/// 工具选择偏好（小写精确匹配，按序）：先只读类（真实 agent 默认免审批），
/// 再命令类；都不命中取工具表首个。**fake 上游专用**（test 模型取首个声明工具）。
const TOOL_PREFERENCE: &[&str] = &[
    "read",
    "glob",
    "grep",
    "bash",
    "shell",
    "run_command",
    "execute_command",
    "terminal",
];

/// 命令类字段的确定性缺省值（`fake-provider-upstream` 的既有契约：
/// 只读命令 → 策略静默放行 → 工具环自动收敛）。
pub const DEFAULT_COMMAND: &str = "echo ok";

/// test 模型侧的确定性命令**前缀**：`mkdir` 不在只读启发式表里（见
/// `sebas_agent::policy::bash_probably_readonly`），因此策略判 `Ask`
/// ——权限卡必弹，而命令本身在会话工作目录里只建一个空目录，无害且确定。
pub const GATED_COMMAND_PREFIX: &str = "mkdir -p .sebas-probe";

/// 请求 `tools` 数组（缺失/非数组 → 空）。
pub fn tools_of(body: Option<&Value>) -> Vec<Value> {
    body.and_then(|v| v.get("tools"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// tool 条目的 JSON Schema（Anthropic `input_schema` / OpenAI `function.parameters`）。
pub fn tool_schema(tool: &Value) -> Option<&Value> {
    tool.get("input_schema")
        .or_else(|| tool.get("function").and_then(|f| f.get("parameters")))
}

/// 请求消息历史里是否已有 `tool_result`（Anthropic content 块）。
pub fn has_tool_result(body: Option<&Value>) -> bool {
    let Some(msgs) = body
        .and_then(|v| v.get("messages"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    msgs.iter().any(|m| {
        m.get("content")
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            })
    })
}

/// 命名 tool 的列表（无名条目丢弃；保序）。
fn named_tools(tools: &[Value]) -> Vec<(String, Option<&Value>)> {
    tools
        .iter()
        .filter_map(|t| {
            let name = t.get("name").and_then(Value::as_str)?;
            Some((name.to_string(), tool_schema(t)))
        })
        .collect()
}

/// fake 上游的 tool 选择：按偏好挑（命中偏好表优先，否则取首个；无合法
/// tools → `None`）。返回 `(name, input)`。
pub fn pick_tool(tools: &[Value]) -> Option<(String, Value)> {
    let named = named_tools(tools);
    if named.is_empty() {
        return None;
    }
    let chosen = TOOL_PREFERENCE
        .iter()
        .find_map(|pref| named.iter().find(|(n, _)| n.to_lowercase() == *pref))
        .unwrap_or(&named[0]);
    let (name, schema) = chosen;
    Some((name.clone(), deterministic_input(*schema)))
}

/// test 模型的 tool 选择：**首个**声明工具（router-core spec「naming the
/// first tool」）。返回 `(name, input)`，input 为可触发审批的确定性对象。
pub fn first_tool(tools: &[Value]) -> Option<(String, Value)> {
    let named = named_tools(tools);
    let (name, schema) = named.first()?;
    Some((name.clone(), deterministic_input_for_approval(*schema)))
}

/// 由 tool 的 `input_schema` 生成**确定性**最小合法 input：只填 `required`
/// 字段，按属性类型/枚举取值（fake 上游的既有语义，命令类字段取
/// [`DEFAULT_COMMAND`]）。缺失 schema / 无 required → `{}`。
pub fn deterministic_input(schema: Option<&Value>) -> Value {
    build_input(schema, DEFAULT_COMMAND)
}

/// 同 [`deterministic_input`]，但命令类字段取 [`GATED_COMMAND_PREFIX`]
/// ——test 模型的权限路径驱动器（策略判 `Ask`）。
pub fn deterministic_input_for_approval(schema: Option<&Value>) -> Value {
    build_input(schema, GATED_COMMAND_PREFIX)
}

/// `test/tools-parallel`：一回合为**每个**声明工具生成 `(name, input)`，
/// input 确定性且**互不相同**（同 schema 的两个 tool 也不会撞）。
///
/// 与 [`pick_tool`] / [`first_tool`] 的有意分岔（design D2）：并行权限请求
/// 需要一回合多个 `tool_use`，fake 上游「只回首个 tool」的规则驱动不了它；
/// 分岔只加不改，fake 侧语义逐字不动。命令类字段带 `-<index>` 后缀
/// （仍非只读 → 各自独立弹卡），其余字符串值按 index 加后缀，
/// 仍撞车时补 `index` 字段兜底。
pub fn parallel_tool_inputs(tools: &[Value]) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    for (index, tool) in tools.iter().enumerate() {
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            continue;
        };
        let command = format!("{GATED_COMMAND_PREFIX}-{index}");
        let mut input = build_input(tool_schema(tool), &command);
        if input.as_object().is_some_and(|o| o.is_empty()) {
            input = json!({ "index": index });
        }
        if out.iter().any(|(_, prev)| *prev == input) {
            if let Some(obj) = input.as_object_mut() {
                obj.insert("index".to_string(), json!(index));
            }
        }
        out.push((name.to_string(), input));
    }
    out
}

/// 内置规则判定（不含文本常量——两处文案不同，判断同规范）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopReply {
    /// 非空 tools 且历史无 tool_result：某个 tool 的 tool_use 轮。
    ToolUse { name: String, input: Value },
    /// 历史已含 tool_result：终文本轮。
    FinalText,
    /// 无 tools：纯文本降级（不报错）。
    PlainText,
}

/// fake 上游的内置规则（[`LoopReply`]；调用方套自己的文案与块 id）。
pub fn loop_reply(body: Option<&Value>) -> LoopReply {
    let tools = tools_of(body);
    if !tools.is_empty()
        && !has_tool_result(body)
        && let Some((name, input)) = pick_tool(&tools)
    {
        return LoopReply::ToolUse { name, input };
    }
    if has_tool_result(body) {
        LoopReply::FinalText
    } else {
        LoopReply::PlainText
    }
}

/// 按 `command` 填命令类字段的 input 构造（`deterministic_input` 系列的实现）。
fn build_input(schema: Option<&Value>, command: &str) -> Value {
    let mut obj = serde_json::Map::new();
    let Some(schema) = schema else {
        return Value::Object(obj);
    };
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let props = schema.get("properties");
    for key in required.iter().filter_map(Value::as_str) {
        let prop = props.and_then(|p| p.get(key));
        let value = if key.to_lowercase().contains("command") {
            Value::String(command.to_string())
        } else {
            deterministic_value(key, prop)
        };
        obj.insert(key.to_string(), value);
    }
    Value::Object(obj)
}

/// 单字段的确定性取值：枚举取首个非 null；其余按类型。
fn deterministic_value(key: &str, prop: Option<&Value>) -> Value {
    // 枚举约束优先：取第一个非 null 值（真实 agent 的 subagent_type 等即此形态）。
    if let Some(first) = prop
        .and_then(|p| p.get("enum"))
        .and_then(Value::as_array)
        .and_then(|e| e.iter().find(|v| !v.is_null()))
    {
        return first.clone();
    }
    let ty = match prop.and_then(|p| p.get("type")) {
        Some(Value::String(s)) => s.as_str(),
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .unwrap_or("string"),
        _ => "string",
    };
    match ty {
        "integer" | "number" => json!(0),
        "boolean" => json!(false),
        "array" => json!([]),
        "object" => json!({}),
        _ if key.to_lowercase().contains("command") => json!(DEFAULT_COMMAND),
        _ => json!("ok"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, required: &[&str]) -> Value {
        let props: serde_json::Map<String, Value> = required
            .iter()
            .map(|k| ((*k).to_string(), json!({"type": "string"})))
            .collect();
        json!({
            "name": name,
            "input_schema": {"type": "object", "properties": props, "required": required},
        })
    }

    fn body_with_tools(tools: Value, tool_result: bool) -> Value {
        let messages = if tool_result {
            json!([
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {}}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok"}]},
            ])
        } else {
            json!([{"role": "user", "content": "hi"}])
        };
        json!({"model": "m", "tools": tools, "messages": messages})
    }

    #[test]
    fn deterministic_input_fills_required_fields_by_type() {
        let schema = json!({
            "type": "object",
            "required": ["path", "count", "flag", "tags", "opts"],
            "properties": {
                "path": {"type": "string"},
                "count": {"type": "integer"},
                "flag": {"type": "boolean"},
                "tags": {"type": "array"},
                "opts": {"type": "object"},
            }
        });
        assert_eq!(
            deterministic_input(Some(&schema)),
            json!({"path": "ok", "count": 0, "flag": false, "tags": [], "opts": {}})
        );
    }

    #[test]
    fn deterministic_input_prefers_enum_first_non_null() {
        let schema = json!({
            "type": "object",
            "required": ["mode"],
            "properties": {"mode": {"type": "string", "enum": [null, "fast", "slow"]}},
        });
        assert_eq!(deterministic_input(Some(&schema)), json!({"mode": "fast"}));
    }

    #[test]
    fn deterministic_input_command_default_matches_fake_contract() {
        let schema = json!({
            "type": "object",
            "required": ["command"],
            "properties": {"command": {"type": "string"}},
        });
        assert_eq!(
            deterministic_input(Some(&schema)),
            json!({"command": "echo ok"}),
            "fake 上游的既有命令契约逐字不变"
        );
    }

    #[test]
    fn approval_input_command_is_not_readonly() {
        let schema = json!({
            "type": "object",
            "required": ["command"],
            "properties": {"command": {"type": "string"}},
        });
        let input = deterministic_input_for_approval(Some(&schema));
        assert_eq!(input, json!({"command": GATED_COMMAND_PREFIX}));
        // 两侧命令的**语义差**（只读 vs 需审批）由 sebas-agent 的策略单测
        // 与 e2e 权限旅程证明；router 不依赖 agent kernel，故此处只钉取值。
        assert_ne!(
            input["command"], DEFAULT_COMMAND,
            "test 模型的确定性命令必须与 fake 的只读命令（echo ok）不同，\
             否则策略静默放行、权限路径驱动不了"
        );
    }

    #[test]
    fn pick_tool_prefers_read_only_then_falls_back_to_first() {
        let tools = vec![tool("Bash", &["command"]), tool("Read", &["path"])];
        assert_eq!(pick_tool(&tools).expect("pick").0, "Read");
        let only = vec![tool("Weird", &["path"])];
        assert_eq!(pick_tool(&only).expect("pick").0, "Weird");
        assert!(pick_tool(&[]).is_none());
        assert!(pick_tool(&[json!({"input_schema": {}})]).is_none());
    }

    #[test]
    fn first_tool_takes_declaration_order_and_gated_input() {
        let tools = vec![tool("Bash", &["command"]), tool("Read", &["path"])];
        let (name, input) = first_tool(&tools).expect("first");
        assert_eq!(name, "Bash");
        assert_eq!(input, json!({"command": GATED_COMMAND_PREFIX}));
    }

    #[test]
    fn loop_reply_truth_table() {
        let tools = json!([tool("Read", &["path"])]);
        // tools 非空 + 无 tool_result → tool_use
        assert!(matches!(
            loop_reply(Some(&body_with_tools(tools.clone(), false))),
            LoopReply::ToolUse { .. }
        ));
        // 有 tool_result → 终文本
        assert_eq!(
            loop_reply(Some(&body_with_tools(tools.clone(), true))),
            LoopReply::FinalText
        );
        // 无 tools → 纯文本
        assert_eq!(
            loop_reply(Some(&json!({"messages": [{"role": "user", "content": "hi"}]}))),
            LoopReply::PlainText
        );
        // 无 tools + 有 tool_result → 终文本
        assert_eq!(
            loop_reply(Some(
                &json!({"messages": [{"role": "user", "content": [{"type": "tool_result", "content": "x"}]}]})
            )),
            LoopReply::FinalText
        );
        // 空 tools 数组 + 有 tool_result → 终文本；无 body → 纯文本
        assert_eq!(
            loop_reply(Some(&body_with_tools(json!([]), true))),
            LoopReply::FinalText
        );
        assert_eq!(loop_reply(None), LoopReply::PlainText);
    }

    #[test]
    fn has_tool_result_scans_all_roles() {
        assert!(!has_tool_result(None));
        assert!(!has_tool_result(Some(&json!({"messages": "nope"}))));
        assert!(has_tool_result(Some(&json!({
            "messages": [{"role": "user", "content": [{"type": "tool_result", "content": "x"}]}]
        }))));
    }

    #[test]
    fn parallel_inputs_are_one_per_tool_and_distinct() {
        let tools = vec![
            tool("Bash", &["command"]),
            tool("Read", &["path"]),
            tool("Write", &["path", "content"]),
            json!({"name": "NoSchema"}),
        ];
        let inputs = parallel_tool_inputs(&tools);
        assert_eq!(inputs.len(), 4, "one entry per declared tool");
        let names: Vec<&str> = inputs.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["Bash", "Read", "Write", "NoSchema"]);
        // 确定性：重复调用结果一致。
        assert_eq!(parallel_tool_inputs(&tools), inputs);
        // 各异：两两不同（含同 schema 的 Read/Write path 值）。
        for i in 0..inputs.len() {
            for j in (i + 1)..inputs.len() {
                assert_ne!(
                    inputs[i].1, inputs[j].1,
                    "inputs must be distinct: {inputs:?}"
                );
            }
        }
        // 命令类字段带 index 后缀，且每个都仍触发审批。
        for (i, (name, input)) in inputs.iter().enumerate() {
            if name == "Bash" {
                let cmd = input["command"].as_str().expect("command");
                assert_eq!(cmd, format!("{GATED_COMMAND_PREFIX}-{i}"));
                assert_ne!(cmd, DEFAULT_COMMAND);
            }
        }
        // 无名条目丢弃。
        assert!(parallel_tool_inputs(&[json!({"input_schema": {}})]).is_empty());
    }

    #[test]
    fn parallel_inputs_dedupe_tools_without_required_fields() {
        let tools = vec![
            json!({"name": "A", "input_schema": {"type": "object"}}),
            json!({"name": "B", "input_schema": {"type": "object"}}),
        ];
        let inputs = parallel_tool_inputs(&tools);
        assert_eq!(inputs.len(), 2);
        assert_ne!(inputs[0].1, inputs[1].1, "{inputs:?}");
        assert_eq!(inputs[0].1, json!({"index": 0}));
        assert_eq!(inputs[1].1, json!({"index": 1}));
    }
}
