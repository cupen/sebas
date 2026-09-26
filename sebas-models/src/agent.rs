//! `AgentRow` — agents 表（settings.db）的 ActiveRecord struct 与
//! AgentConfig 等价载荷（`AgentDefinition`）的互转。
//!
//! add-agent-settings-and-session-titles 决策 1：settings.db 的 `agents`
//! 表是 agent 目录唯一运行时权威（config.toml `[acp.agents.*]` 降级为种子
//! 源）。列集镜像 `AgentConfig` 可承载字段：
//!
//! - `driver`：封闭标签 `claude` | `acp`（配置层概念，不上 `/api/agents`
//!   的 catalog wire——管理面快照除外）；
//! - `path` + `args`：launch 定义。claude → 可执行文件 + 追加参数；acp →
//!   `path` 存 command[0]、`args` 存其余 argv（单一定义形状，无第二套列）；
//! - `models`：claude 驱动的模型别名表覆盖（JSON 文本；可空 = 内置表）；
//! - `startup_timeout_secs` / `idle_kill_secs` / `work_dir` / `display`：
//!   与 config 同名键同义；
//! - `source`：行来源 `seed`（config 种子导入）| `ui`（Settings 创建），
//!   纯呈现信息，不影响行为；
//! - claude 变体的 `sessions_dir` 不入表（UI 管理场景不需要，缺省走既有
//!   默认——决策 1 原文）。
//!
//! PRIMARY KEY 等约束只在根 crate 注册表（SETTINGS_TABLES）的 DDL 里表达。

use sebas_schema_derive::{ActiveRecord, SchemaColumns};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// agents 行。读取路径可能只取部分列，其余字段仅为 schema 声明存在，
/// 故整体 allow(dead_code)。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, SchemaColumns, ActiveRecord)]
#[active_record(table = "agents")]
#[active_record(pk = "id")]
pub struct AgentRow {
    /// agent id（config 键 / catalog wire id）。`native` 是内置内核保留
    /// id，不入表（mutation 层拒绝）。
    pub id: String,
    /// 驱动标签：`claude` | `acp`。
    pub driver: String,
    /// launch 可执行文件：claude 的 `path`，或 acp command 的 argv[0]。
    pub path: Option<String>,
    /// 追加 argv（JSON 数组文本）；acp 行 = command[1..]。
    pub args: Option<String>,
    /// 产品展示名（可空 = 缺省回退 id 本身）。
    pub display: Option<String>,
    /// claude 驱动的模型别名表覆盖（JSON 数组文本；可空 = 内置表）。
    pub models: Option<String>,
    #[column(default = "30")]
    pub startup_timeout_secs: i64,
    #[column(default = "172800")]
    pub idle_kill_secs: i64,
    /// 会话工作目录覆盖（可空）。
    pub work_dir: Option<String>,
    /// 行来源：`seed`（config 种子导入）| `ui`（Settings 创建）。
    #[column(default = "seed")]
    pub source: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `AgentConfig` 等价载荷（launch 定义，不含 id / 来源 / 时间戳）：种子
/// 导入（根 crate 的 `AgentConfig` → 这里）与 agents 域 mutation 的校验
/// 形状共用。`driver` 只认 `claude` | `acp`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub driver: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout_secs: u64,
    #[serde(default = "default_idle_kill")]
    pub idle_kill_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
}

fn default_startup_timeout() -> u64 {
    30
}
fn default_idle_kill() -> u64 {
    172800
}

/// 与根 crate `config.rs` 的缺省值同源（`default_claude_path`）。
pub const DEFAULT_CLAUDE_PATH: &str = "claude";

/// agents 表行上保留的内置内核 id（mutation 层拒绝 put/delete）。
pub const RESERVED_NATIVE_ID: &str = "native";

impl Default for AgentDefinition {
    fn default() -> Self {
        Self {
            driver: "claude".to_string(),
            path: None,
            args: Vec::new(),
            display: None,
            models: None,
            startup_timeout_secs: default_startup_timeout(),
            idle_kill_secs: default_idle_kill(),
            work_dir: None,
        }
    }
}

/// 驱动标签的封闭集（决策 4：驱动实现仍封闭于 `claude` | `acp`；opencode
/// 是表单预设，存储为 `driver=acp` + command argv）。
pub fn is_valid_driver(driver: &str) -> bool {
    driver == "claude" || driver == "acp"
}

impl AgentDefinition {
    /// launch argv：claude → `[path, args..]`；acp → `[path, args..]`（path
    /// 即 command[0]）。`path` 缺省时 claude 回退 `"claude"`、acp 视为空
    /// argv（catalog 探测报 `empty command`）。
    pub fn command(&self) -> Vec<String> {
        let head = self
            .path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if self.driver == "claude" {
                    DEFAULT_CLAUDE_PATH.to_string()
                } else {
                    String::new()
                }
            });
        let mut v = vec![head];
        v.extend(self.args.iter().cloned());
        if v[0].is_empty() {
            v.remove(0);
        }
        v
    }

    /// claude 驱动的生效模型别名表：非空覆盖优先，空/缺省 = `None`（调用
    /// 方回退内置表——与 config `resolved_models` 同语义）。
    pub fn resolved_models(&self) -> Option<Vec<String>> {
        match self.models.as_deref() {
            Some(list) if !list.is_empty() => Some(list.to_vec()),
            _ => None,
        }
    }
}

impl AgentRow {
    /// 载荷 → 行（写入路径）。`created_at`/`updated_at` 取当前时间；
    /// source 由调用方给定（`seed` | `ui`）。
    pub fn from_definition(id: &str, def: &AgentDefinition, source: &str) -> Self {
        let now = sebas_domain::prim::now_unix();
        Self {
            id: id.to_string(),
            driver: def.driver.clone(),
            path: def.path.clone(),
            args: json_text(&def.args),
            display: def.display.clone(),
            models: def.models.as_ref().and_then(|m| json_text(m)),
            startup_timeout_secs: def.startup_timeout_secs as i64,
            idle_kill_secs: def.idle_kill_secs as i64,
            work_dir: def.work_dir.clone(),
            source: source.to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    /// 行 → 载荷（读取路径）。
    pub fn to_definition(&self) -> AgentDefinition {
        AgentDefinition {
            driver: self.driver.clone(),
            path: self.path.clone(),
            args: parse_json_list(self.args.as_deref()).unwrap_or_default(),
            display: self.display.clone(),
            models: parse_json_list(self.models.as_deref()),
            startup_timeout_secs: self.startup_timeout_secs.max(0) as u64,
            idle_kill_secs: self.idle_kill_secs.max(0) as u64,
            work_dir: self.work_dir.clone(),
        }
    }

    /// 行 → agents 域快照条目（管理面 wire 形状）：launch 槽位与 put 载荷
    /// 同词表（args/models 是字符串数组），外加 id / 来源 / 时间戳。调用方
    /// 可按同一形状直接回填编辑表单（put 整体替换的回填视图）。
    pub fn to_item(&self) -> serde_json::Map<String, Value> {
        let mut item = serde_json::Map::new();
        item.insert("id".into(), Value::String(self.id.clone()));
        item.insert("driver".into(), Value::String(self.driver.clone()));
        if let Some(p) = &self.path {
            item.insert("path".into(), Value::String(p.clone()));
        }
        if let Some(args) = parse_json_list(self.args.as_deref())
            && !args.is_empty()
        {
            item.insert(
                "args".into(),
                Value::Array(args.into_iter().map(Value::String).collect()),
            );
        }
        if let Some(d) = &self.display {
            item.insert("display".into(), Value::String(d.clone()));
        }
        if let Some(models) = parse_json_list(self.models.as_deref())
            && !models.is_empty()
        {
            item.insert(
                "models".into(),
                Value::Array(models.into_iter().map(Value::String).collect()),
            );
        }
        item.insert(
            "startup_timeout_secs".into(),
            Value::from(self.startup_timeout_secs.max(0)),
        );
        item.insert(
            "idle_kill_secs".into(),
            Value::from(self.idle_kill_secs.max(0)),
        );
        if let Some(w) = &self.work_dir {
            item.insert("work_dir".into(), Value::String(w.clone()));
        }
        item.insert("source".into(), Value::String(self.source.clone()));
        item.insert("created_at".into(), Value::from(self.created_at));
        item.insert("updated_at".into(), Value::from(self.updated_at));
        item
    }
}

/// `Option<Vec<String>>` → JSON 数组文本（空表与缺省都不落列——缺字段 =
/// 未覆盖）。
fn json_text(list: &[String]) -> Option<String> {
    if list.is_empty() {
        return None;
    }
    serde_json::to_string(list).ok()
}

/// JSON 数组文本 → 字符串列表（形状不符 = 未覆盖语义的 `None`）。
fn parse_json_list(text: Option<&str>) -> Option<Vec<String>> {
    let text = text?;
    let v: Value = serde_json::from_str(text).ok()?;
    let arr = v.as_array()?;
    Some(
        arr.iter()
            .filter_map(|e| e.as_str().map(str::to_string))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sebas_db::schema::{TableSchema, open_and_sync};

    fn definition(driver: &str, path: Option<&str>, args: &[&str]) -> AgentDefinition {
        AgentDefinition {
            driver: driver.to_string(),
            path: path.map(str::to_string),
            args: args.iter().map(|a| a.to_string()).collect(),
            display: None,
            models: None,
            startup_timeout_secs: 30,
            idle_kill_secs: 172800,
            work_dir: None,
        }
    }

    /// row ↔ 载荷往返：launch 定义（path/args/display/models/work_dir）逐
    /// 字段保真。
    #[test]
    fn row_definition_round_trip_preserves_launch_fields() {
        let def = AgentDefinition {
            driver: "claude".into(),
            path: Some("/usr/local/bin/claude".into()),
            args: vec!["--model".into(), "sonnet".into()],
            display: Some("My Claude".into()),
            models: Some(vec!["sonnet".into(), "opus[1m]".into()]),
            startup_timeout_secs: 45,
            idle_kill_secs: 0,
            work_dir: Some("/tmp/work".into()),
        };
        let row = AgentRow::from_definition("myclaude", &def, "seed");
        assert_eq!(row.id, "myclaude");
        assert_eq!(row.source, "seed");
        assert_eq!(row.path.as_deref(), Some("/usr/local/bin/claude"));
        let back = row.to_definition();
        assert_eq!(back, def, "载荷经行往返逐字段一致");
    }

    /// 缺省槽位不落列：空 args/models 读回为空/None（缺字段 = 未配置）；
    /// 非空 argv 以 JSON 文本进列并按 argv 读回。
    #[test]
    fn absent_list_slots_stay_absent() {
        let row = AgentRow::from_definition("bare", &definition("acp", Some("opencode"), &[]), "ui");
        assert_eq!(row.source, "ui");
        assert!(row.args.is_none(), "空 argv 不落列");
        assert!(row.models.is_none());
        let back = row.to_definition();
        assert!(back.args.is_empty());
        assert_eq!(back.models, None);
        assert_eq!(back.command(), vec!["opencode".to_string()]);

        let with_args =
            AgentRow::from_definition("oc", &definition("acp", Some("opencode"), &["acp"]), "ui");
        assert_eq!(with_args.args.as_deref(), Some(r#"["acp"]"#));
        assert_eq!(
            with_args.to_definition().command(),
            vec!["opencode".to_string(), "acp".to_string()]
        );
    }

    /// launch argv 形状：claude 缺 path 回退内置 `"claude"`；acp 缺 path →
    /// 空 argv。
    #[test]
    fn command_shapes_follow_driver() {
        let mut def = definition("claude", None, &["--model", "opus"]);
        assert_eq!(def.command(), vec!["claude".to_string(), "--model".into(), "opus".into()]);
        def.path = Some("  ".into());
        assert_eq!(def.command()[0], "claude", "空白 path 视同缺省");
        let acp = definition("acp", None, &[]);
        assert!(acp.command().is_empty(), "acp 无 command[0] = 空 argv");
        assert_eq!(AgentRow::from_definition("x", &definition("acp", Some("cursor-agent"), &["acp"]), "ui").to_definition().command(),
            vec!["cursor-agent".to_string(), "acp".to_string()]);
    }

    /// 驱动标签封闭集 + 保留 id。
    #[test]
    fn driver_tag_set_is_closed() {
        assert!(is_valid_driver("claude"));
        assert!(is_valid_driver("acp"));
        assert!(!is_valid_driver("gemini"));
        assert_eq!(RESERVED_NATIVE_ID, "native");
    }

    /// 快照条目投影：launch 槽位与 put 载荷同词表（数组保持数组），身份与
    /// 账目字段（id/source/created_at/updated_at）一并下发；空数组槽位不产
    /// 生键（缺字段 = 未配置）。
    #[test]
    fn to_item_projects_the_management_wire_shape() {
        let def = AgentDefinition {
            driver: "acp".into(),
            path: Some("opencode".into()),
            args: vec!["acp".into()],
            display: Some("OpenCode".into()),
            models: None,
            startup_timeout_secs: 60,
            idle_kill_secs: 3600,
            work_dir: None,
        };
        let item = AgentRow::from_definition("opencode", &def, "ui").to_item();
        assert_eq!(item.get("id").and_then(Value::as_str), Some("opencode"));
        assert_eq!(item.get("driver").and_then(Value::as_str), Some("acp"));
        assert_eq!(item.get("path").and_then(Value::as_str), Some("opencode"));
        assert_eq!(
            item.get("args").and_then(Value::as_array).map(Vec::len),
            Some(1),
            "args 以字符串数组上 wire"
        );
        assert_eq!(item.get("display").and_then(Value::as_str), Some("OpenCode"));
        assert!(item.get("models").is_none(), "空槽位不产键");
        assert_eq!(item.get("source").and_then(Value::as_str), Some("ui"));
        assert!(item.get("created_at").is_some() && item.get("updated_at").is_some());
    }

    /// 生效模型别名表：非空覆盖优先；空表/缺省 = None（回退内置）。
    #[test]
    fn resolved_models_follows_override_semantics() {
        let mut def = definition("claude", Some("claude"), &[]);
        assert_eq!(def.resolved_models(), None, "缺省回退内置");
        def.models = Some(vec![]);
        assert_eq!(def.resolved_models(), None, "空表同缺省");
        def.models = Some(vec!["sonnet".into()]);
        assert_eq!(def.resolved_models(), Some(vec!["sonnet".to_string()]));
    }

    /// 存储契约：列清单钉死（列名与根 crate SETTINGS_TABLES 的 DDL 一致）；
    /// ActiveRecord 生成的 save/find/all/delete 经 SQLite 往返全等。
    #[test]
    fn typed_row_round_trips_through_sqlite() {
        use sebas_db::record::Record;
        assert_eq!(
            AgentRow::COLUMNS,
            &[
                "id",
                "driver",
                "path",
                "args",
                "display",
                "models",
                "startup_timeout_secs",
                "idle_kill_secs",
                "work_dir",
                "source",
                "created_at",
                "updated_at",
            ]
        );
        assert_eq!(AgentRow::PK_COLUMNS, &["id"]);
        assert_eq!(AgentRow::TABLE, "agents");

        static TABLES: &[TableSchema] = &[TableSchema {
            name: "agents",
            create_table_ddl: "CREATE TABLE agents (
                id                   TEXT PRIMARY KEY,
                driver               TEXT NOT NULL,
                path                 TEXT,
                args                 TEXT,
                display              TEXT,
                models               TEXT,
                startup_timeout_secs INTEGER NOT NULL DEFAULT 30,
                idle_kill_secs       INTEGER NOT NULL DEFAULT 172800,
                work_dir             TEXT,
                source               TEXT NOT NULL DEFAULT 'seed',
                created_at           INTEGER NOT NULL,
                updated_at           INTEGER NOT NULL
            );",
            index_ddls: &[],
            columns: AgentRow::schema_columns(),
        }];
        let dir = tempfile::tempdir().unwrap();
        let conn = open_and_sync(&dir.path().join("s.db"), TABLES).unwrap().0;

        let def = AgentDefinition {
            driver: "acp".into(),
            path: Some("opencode".into()),
            args: vec!["acp".into()],
            display: Some("OpenCode".into()),
            models: None,
            startup_timeout_secs: 60,
            idle_kill_secs: 3600,
            work_dir: Some("/tmp/oc".into()),
        };
        let row = AgentRow::from_definition("opencode", &def, "ui");
        row.save(&conn).unwrap();
        let found = AgentRow::find(&conn, "opencode").unwrap().unwrap();
        assert_eq!(found, row, "行经 SQLite 存取逐字段全等");
        assert_eq!(found.to_definition(), def);

        // upsert 覆盖同 id 行。
        let mut edited = found.clone();
        edited.display = Some("Renamed".into());
        edited.save(&conn).unwrap();
        assert_eq!(AgentRow::all(&conn).unwrap().len(), 1, "同 id upsert 不产生第二行");

        AgentRow::delete(&conn, "opencode").unwrap();
        assert!(AgentRow::find(&conn, "opencode").unwrap().is_none());
    }
}
