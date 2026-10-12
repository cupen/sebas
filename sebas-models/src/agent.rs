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
//! - claude 变体的 `sessions_dir` 入表（fix-webui-qa-round2 2.4 解除早期
//!   「不入表」决策：Settings 表单要能建出 spawn 可用的 agent——sessions
//!   目录是 spawn 关键字段；可空 = 走既有默认，旧行为零迁移语义）。
//!
//! PRIMARY KEY 等约束只在根 crate 注册表（SETTINGS_TABLES）的 DDL 里表达。

use sebas_schema_derive::{ActiveRecord, SchemaColumns};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::str::FromStr;

/// claude 模型档位（add-claude-model-slots D1）：固定四档，声明序即强弱序
/// （`fable` > `opus` > `sonnet` > `haiku`）。档位是封闭枚举——claude CLI 只
/// 认内置别名，自定义档名选了解析不了（proposal Non-goals）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModelSlot {
    Fable,
    Opus,
    Sonnet,
    Haiku,
}

impl ModelSlot {
    /// 全部档位，按强弱序（BTreeMap 迭代序同此）。
    pub const ALL: [ModelSlot; 4] = [
        ModelSlot::Fable,
        ModelSlot::Opus,
        ModelSlot::Sonnet,
        ModelSlot::Haiku,
    ];

    /// wire / 存储词。
    pub const fn as_str(self) -> &'static str {
        match self {
            ModelSlot::Fable => "fable",
            ModelSlot::Opus => "opus",
            ModelSlot::Sonnet => "sonnet",
            ModelSlot::Haiku => "haiku",
        }
    }

    /// `ANTHROPIC_DEFAULT_<SUFFIX>_MODEL` 的 `<SUFFIX>`。
    pub const fn env_suffix(self) -> &'static str {
        match self {
            ModelSlot::Fable => "FABLE",
            ModelSlot::Opus => "OPUS",
            ModelSlot::Sonnet => "SONNET",
            ModelSlot::Haiku => "HAIKU",
        }
    }
}

impl std::fmt::Display for ModelSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// wire 词 → 档位。未知档名拒绝（typed error 含原词）。
impl FromStr for ModelSlot {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fable" => Ok(ModelSlot::Fable),
            "opus" => Ok(ModelSlot::Opus),
            "sonnet" => Ok(ModelSlot::Sonnet),
            "haiku" => Ok(ModelSlot::Haiku),
            other => Err(format!(
                "未知模型档位 '{other}'（只认 fable / opus / sonnet / haiku）"
            )),
        }
    }
}

impl Serialize for ModelSlot {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ModelSlot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        let s = String::deserialize(deserializer)?;
        ModelSlot::from_str(&s).map_err(D::Error::custom)
    }
}

/// claude agent 的模型档位映射（add-claude-model-slots D1）：启用档位 → 目录
/// 模型 id（provider 模型与 router 别名同权），外加一个默认档标记。
///
/// wire / 存储形状是一个扁平对象：`{"opus": "<id>", ..., "default": "<slot>"}`。
/// 不变量（构造即校验，反序列化同样强制）：至少启用一个档位；`default_slot`
/// 必须落在启用档集内。空映射在 mutation 面归一为 `None`（等价未配置）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSlotMapping {
    /// 启用档位 → 目录模型 id（BTreeMap 迭代序 = 强弱序）。
    pub slots: BTreeMap<ModelSlot, String>,
    /// 默认档（校验保证 ∈ `slots` 键集）。
    pub default_slot: ModelSlot,
}

impl ModelSlotMapping {
    /// 构造 + 校验（供程序化装配与测试用；wire 反序列化走同一不变量）。
    pub fn new(
        slots: BTreeMap<ModelSlot, String>,
        default_slot: ModelSlot,
    ) -> Result<Self, String> {
        if slots.is_empty() {
            return Err("模型档位映射至少启用一个档位".to_string());
        }
        if !slots.contains_key(&default_slot) {
            return Err(format!(
                "默认档 '{}' 未启用（default 必须落在启用档集内）",
                default_slot
            ));
        }
        Ok(Self {
            slots,
            default_slot,
        })
    }

    /// 默认档的模型 id（spawn 时 `ANTHROPIC_MODEL` 的取值）。
    pub fn default_model(&self) -> Option<&str> {
        self.slots.get(&self.default_slot).map(String::as_str)
    }

    /// 最弱启用档的模型 id（`CLAUDE_CODE_SUBAGENT_MODEL` 的取值）。
    pub fn weakest_model(&self) -> Option<&str> {
        self.slots.values().next_back().map(String::as_str)
    }

    /// 会话模型词汇（add-claude-model-slots D3）：`default` + 启用档（强弱序）。
    pub fn resolved_aliases(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.slots.len() + 1);
        out.push(DEFAULT_CLAUDE_MODEL_ALIAS.to_string());
        out.extend(self.slots.keys().map(|s| s.as_str().to_string()));
        out
    }
}

/// 会话模型词汇里的显式缺省条目（与 `sebas_acp::claude::DEFAULT_CLAUDE_MODEL`
/// 同词——两处都是「回到 CLI 默认模型」的语义，此处避免 sebas-models 依赖
/// sebas-acp 而本地镜像一份常量）。
pub const DEFAULT_CLAUDE_MODEL_ALIAS: &str = "default";

impl Serialize for ModelSlotMapping {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap as _;
        let mut map = serializer.serialize_map(Some(self.slots.len() + 1))?;
        for (slot, model) in &self.slots {
            map.serialize_entry(slot.as_str(), model)?;
        }
        map.serialize_entry("default", self.default_slot.as_str())?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for ModelSlotMapping {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        let v = Value::deserialize(deserializer)?;
        let obj = match v {
            Value::Object(map) => map,
            _ => {
                return Err(D::Error::custom(
                    "models 期望结构化映射对象（档位名 fable/opus/sonnet/haiku + default 键），\
                     旧裸数组形态已不再支持",
                ));
            }
        };
        let mut slots: BTreeMap<ModelSlot, String> = BTreeMap::new();
        let mut default_slot: Option<ModelSlot> = None;
        for (key, val) in obj {
            if key == "default" {
                let s = val.as_str().ok_or_else(|| {
                    D::Error::custom("models.default 必须是档位名字符串")
                })?;
                default_slot = Some(ModelSlot::from_str(s).map_err(D::Error::custom)?);
                continue;
            }
            let slot = ModelSlot::from_str(&key).map_err(D::Error::custom)?;
            let model = val
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    D::Error::custom(format!("models.{key} 必须是非空模型 id 字符串"))
                })?;
            slots.insert(slot, model.to_string());
        }
        let default_slot = default_slot.ok_or_else(|| {
            D::Error::custom("models 缺少 default 键（须点名一个启用档为默认）")
        })?;
        ModelSlotMapping::new(slots, default_slot).map_err(D::Error::custom)
    }
}

/// 校验 driver 与模型的组合（add-claude-model-slots D6）：`models` 仅 claude
/// driver 可携带，其余 driver 携带即拒。config 种子导入与 mutation 面共用本
/// 函数，避免两套口径。
pub fn validate_models_for_driver(
    driver: &str,
    models: &Option<ModelSlotMapping>,
) -> Result<(), String> {
    if models.is_some() && driver != "claude" {
        return Err(format!(
            "driver '{driver}' 不支持模型档位映射（仅 claude 可携带 models）"
        ));
    }
    Ok(())
}

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
    /// claude 驱动的模型档位映射（JSON 对象文本；可空 = 未配置 = 内置表）。
    /// （add-claude-model-slots D1）列名与 DDL 不变，形状从 `Vec<String>`
    /// 换成结构化映射；旧 `Vec` 值读取时宽容降级为 `None`（视为未配置）。
    pub models: Option<String>,
    #[column(default = "30")]
    pub startup_timeout_secs: i64,
    #[column(default = "172800")]
    pub idle_kill_secs: i64,
    /// 会话工作目录覆盖（可空）。
    pub work_dir: Option<String>,
    /// （fix-webui-qa-round2 2.4，M-A4）claude 驱动的 sessions 目录覆盖
    /// （可空 = 走既有默认）。此前决策「不入表」随 GUI 建目录能力一并
    /// 解除：表单建出的 agent 要能配到可用形态（spec「Agent form covers
    /// spawn-critical fields」），sessions 目录是 spawn 关键字段之一。
    pub sessions_dir: Option<String>,
    /// 行来源：`seed`（config 种子导入）| `ui`（Settings 创建）。
    #[column(default = "seed")]
    pub source: String,
    /// 删除墓碑（0 = 活跃，1 = 已删除）：行保留使 config 种子导入天然跳过
    /// 已删 id，删除对 catalog / spawn / 重启回填全部生效（spec「disappears
    /// from the catalog」）。put 同 id = 复活（deleted 归 0）。
    #[column(default = "0")]
    pub deleted: i64,
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
    /// （add-claude-model-slots D1）claude 驱动的模型档位映射；`None` =
    /// 未配置（回退内置别名表）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<ModelSlotMapping>,
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout_secs: u64,
    #[serde(default = "default_idle_kill")]
    pub idle_kill_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
    /// （fix-webui-qa-round2 2.4，M-A4）claude 驱动的 sessions 目录覆盖。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sessions_dir: Option<String>,
}

fn default_startup_timeout() -> u64 {
    30
}
fn default_idle_kill() -> u64 {
    172800
}

/// 与根 crate `config.rs` 的缺省值同源（`default_claude_path`）。
pub const DEFAULT_CLAUDE_PATH: &str = "claude";

/// 与根 crate `config.rs` 的缺省值同源（`default_pi_path`）。
pub const DEFAULT_PI_PATH: &str = "pi";

/// 与根 crate `config.rs` 的缺省值同源（`default_pi_sessions_dir`）。
pub const DEFAULT_PI_SESSIONS_DIR: &str = "~/.pi/agent/sessions";

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
            sessions_dir: None,
        }
    }
}

/// 驱动标签的封闭集（决策 4：驱动实现仍封闭于 `claude` | `acp`；opencode
/// 是表单预设，存储为 `driver=acp` + command argv。add-pi-driver 追加 `pi`
/// ——pi 走自有 headless RPC，不是通用 ACP）。
pub fn is_valid_driver(driver: &str) -> bool {
    driver == "claude" || driver == "acp" || driver == "pi"
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
                } else if self.driver == "pi" {
                    DEFAULT_PI_PATH.to_string()
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

    /// （add-claude-model-slots D3）claude 驱动的生效模型词汇：有映射 →
    /// `default` + 启用档（强弱序）；无映射 → `None`（调用方回退内置表）。
    pub fn resolved_models(&self) -> Option<Vec<String>> {
        self.models.as_ref().map(ModelSlotMapping::resolved_aliases)
    }
}

impl AgentRow {
    /// 墓碑判定（删除后、复活前的行）。
    pub fn is_deleted(&self) -> bool {
        self.deleted != 0
    }

    /// agents 域快照形状（core channel 服务端与 webui InProcessBackend 共
    /// 用一处，避免两处漂移）：`agents` 只含活跃行（墓碑对普通消费方不可
    /// 见），`deleted_ids` 是墓碑 id 列表——catalog union 用它把 config 侧
    /// 同 id 条目一并排除，否则被删的种子 agent 会在读取期被 config 并回。
    pub fn snapshot_value(rows: &[AgentRow]) -> Value {
        let active: Vec<Value> = rows
            .iter()
            .filter(|r| !r.is_deleted())
            .map(|r| Value::Object(r.to_item()))
            .collect();
        let deleted_ids: Vec<Value> = rows
            .iter()
            .filter(|r| r.is_deleted())
            .map(|r| Value::String(r.id.clone()))
            .collect();
        serde_json::json!({ "agents": active, "deleted_ids": deleted_ids })
    }

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
            models: def
                .models
                .as_ref()
                .and_then(|m| serde_json::to_string(m).ok()),
            startup_timeout_secs: def.startup_timeout_secs as i64,
            idle_kill_secs: def.idle_kill_secs as i64,
            work_dir: def.work_dir.clone(),
            sessions_dir: def.sessions_dir.clone(),
            source: source.to_string(),
            deleted: 0,
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
            models: parse_models(self.models.as_deref()),
            startup_timeout_secs: self.startup_timeout_secs.max(0) as u64,
            idle_kill_secs: self.idle_kill_secs.max(0) as u64,
            work_dir: self.work_dir.clone(),
            sessions_dir: self.sessions_dir.clone(),
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
        if let Some(mapping) = parse_models(self.models.as_deref())
            && let Ok(v) = serde_json::to_value(&mapping)
        {
            item.insert("models".into(), v);
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
        if let Some(sd) = &self.sessions_dir {
            item.insert("sessions_dir".into(), Value::String(sd.clone()));
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

/// `models` 列 JSON 文本 → 档位映射（add-claude-model-slots D1）。旧
/// `Vec<String>` 形态（历史别名表）宽容降级为 `None`——它从未驱动 env，
/// 降级只丢「词汇表」而非配置；形状非法同样降级为 `None`（warn 一条）。
fn parse_models(text: Option<&str>) -> Option<ModelSlotMapping> {
    let text = text?;
    match serde_json::from_str::<ModelSlotMapping>(text) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!(
                stored = %text,
                error = %e,
                "agents.models 存量值不是结构化档位映射（旧裸数组形态？）：视为未配置"
            );
            None
        }
    }
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
            sessions_dir: None,
        }
    }

    fn mapping(slots: &[(ModelSlot, &str)], default_slot: ModelSlot) -> ModelSlotMapping {
        ModelSlotMapping::new(
            slots.iter().map(|(s, m)| (*s, m.to_string())).collect(),
            default_slot,
        )
        .expect("test mapping is valid")
    }

    /// row ↔ 载荷往返：launch 定义（path/args/display/models/work_dir）逐
    /// 字段保真。
    #[test]
    fn row_definition_round_trip_preserves_launch_fields() {
        let def = AgentDefinition {
            driver: "claude".into(),
            path: Some("/usr/local/bin/claude".into()),
            args: vec!["--scenario".into(), "thinking".into()],
            display: Some("My Claude".into()),
            models: Some(mapping(
                &[(ModelSlot::Sonnet, "sonnet"), (ModelSlot::Opus, "opus[1m]")],
                ModelSlot::Sonnet,
            )),
            startup_timeout_secs: 45,
            idle_kill_secs: 0,
            work_dir: Some("/tmp/work".into()),
            sessions_dir: None,
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
            sessions_dir: None,
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

    /// （add-claude-model-slots 1.1）生效模型词汇新语义：有映射 → `default`
    /// + 启用档（强弱序）；无映射 → None（回退内置表）。
    #[test]
    fn resolved_models_follows_mapping_semantics() {
        let mut def = definition("claude", Some("claude"), &[]);
        assert_eq!(def.resolved_models(), None, "无映射回退内置");
        def.models = Some(mapping(
            &[(ModelSlot::Sonnet, "sonnet-x"), (ModelSlot::Opus, "opus-y")],
            ModelSlot::Sonnet,
        ));
        assert_eq!(
            def.resolved_models(),
            Some(vec![
                "default".to_string(),
                "opus".to_string(),
                "sonnet".to_string(),
            ]),
            "有映射 = default + 启用档（强弱序，与插入序无关）"
        );
    }

    /// （1.1）档位映射不变量：default 必须落在启用档集内；空映射被拒。
    #[test]
    fn mapping_rejects_default_outside_enabled_slots() {
        let err = ModelSlotMapping::new(
            [(ModelSlot::Sonnet, "s".to_string())].into_iter().collect(),
            ModelSlot::Haiku,
        )
        .expect_err("default 落在停用档必须被拒");
        assert!(err.contains("haiku"), "错误须点名越界档位：{err}");
        let err = ModelSlotMapping::new(BTreeMap::new(), ModelSlot::Opus)
            .expect_err("空映射必须被拒");
        assert!(err.contains("至少启用"), "{err}");
    }

    /// （1.1）wire 形状：扁平对象 `{slot: id, default: slot}` 往返；未知档名
    /// 与缺 default 键被拒。
    #[test]
    fn mapping_serializes_as_flat_object_and_rejects_bad_shape() {
        let m = mapping(
            &[(ModelSlot::Fable, "f"), (ModelSlot::Haiku, "h")],
            ModelSlot::Haiku,
        );
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v, serde_json::json!({"fable": "f", "haiku": "h", "default": "haiku"}));
        let back: ModelSlotMapping = serde_json::from_value(v).unwrap();
        assert_eq!(back, m);

        // 旧裸数组形态在结构化反序列化下被拒（mutation/wire 面）。
        assert!(serde_json::from_str::<ModelSlotMapping>(r#"["opus","sonnet"]"#).is_err());
        // 未知档名。
        let err = serde_json::from_str::<ModelSlotMapping>(r#"{"bogus":"m","default":"bogus"}"#)
            .expect_err("未知档名必须被拒");
        assert!(err.to_string().contains("bogus"), "{err}");
        // 缺 default 键。
        assert!(serde_json::from_str::<ModelSlotMapping>(r#"{"opus":"m"}"#).is_err());
        // default 指向未启用档。
        assert!(
            serde_json::from_str::<ModelSlotMapping>(r#"{"opus":"m","default":"haiku"}"#).is_err()
        );
    }

    /// （1.1）旧 `Vec<String>` 存量值宽容降级为 None（视为未配置，不炸读
    /// 路径）；写入只写新形状。
    #[test]
    fn legacy_vec_value_degrades_to_none_on_read() {
        let mut row = AgentRow::from_definition("c", &definition("claude", Some("claude"), &[]), "seed");
        row.models = Some(r#"["opus","sonnet"]"#.to_string());
        assert_eq!(row.to_definition().models, None, "旧裸数组读为未配置");
        assert!(row.to_item().get("models").is_none(), "降级后不产 models 键");

        // 新形状写入是对象文本。
        let mut def = definition("claude", Some("claude"), &[]);
        def.models = Some(mapping(&[(ModelSlot::Opus, "opus-x")], ModelSlot::Opus));
        let row = AgentRow::from_definition("c2", &def, "ui");
        assert_eq!(row.models.as_deref(), Some(r#"{"opus":"opus-x","default":"opus"}"#));
        assert_eq!(row.to_definition().models, def.models);
    }

    /// （1.3/D6）models 仅 claude driver 可携带。
    #[test]
    fn models_are_claude_only() {
        let m = mapping(&[(ModelSlot::Opus, "o")], ModelSlot::Opus);
        assert!(validate_models_for_driver("claude", &Some(m.clone())).is_ok());
        assert!(validate_models_for_driver("acp", &Some(m.clone())).is_err());
        assert!(validate_models_for_driver("pi", &Some(m)).is_err());
        assert!(validate_models_for_driver("acp", &None).is_ok());
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
                "sessions_dir",
                "source",
                "deleted",
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
                sessions_dir         TEXT,
                source               TEXT NOT NULL DEFAULT 'seed',
                deleted              INTEGER NOT NULL DEFAULT 0,
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
            sessions_dir: None,
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
