//! `ProviderRow` / `ModelAliasRow` — providers 与 model_aliases 表的
//! ActiveRecord struct + 按非键列的域查询。
//!
//! providers 表是**类型化列**（single-state-dir D3）：一行即一个 provider
//! 实例，`config TEXT (JSON blob)` 的无类型存储已终结。列名与既有 provider
//! JSON 键逐一对齐（`name` / `preset` / 三个 base_url 槽位 / `api_key` /
//! `api_key_env` / `default_model` / `protocol` / `models` / `model_map`），
//! channel 上的 provider JSON 形状经 [`ProviderRow::to_item`] /
//! [`ProviderRow::from_item`] 的 serde 转换保持不变（webui / router 的线
//! 形状零变化；两个 `Vec`/`Map` 形字段以 JSON 文本进列）。
//!
//! `Item = Map<String, Value>` 不再出现在存储路径：`from_item` / `to_item`
//! 是它与行 struct 之间的唯一边界（存储侧只见 `ProviderRow`）。

use sebas_db::rusqlite::Connection;
use sebas_schema_derive::{ActiveRecord, SchemaColumns};
use serde_json::Value;

/// providers 行。PRIMARY KEY 等约束只在根 crate 注册表的 DDL 里表达。
/// 读取路径可能只取部分列，其余字段仅为 schema 声明存在, 故整体
/// allow(dead_code)。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, SchemaColumns, ActiveRecord)]
#[active_record(table = "providers")]
#[active_record(pk = "id")]
pub struct ProviderRow {
    /// provider 实例名（= 既有 Item 映射的键）。
    pub id: String,
    /// 卡片表单写入的展示名（Item 的 `name`）。
    pub name: Option<String>,
    pub preset: Option<String>,
    pub base_url_anthropic: Option<String>,
    pub base_url_openai_chat: Option<String>,
    pub base_url_openai_responses: Option<String>,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub default_model: Option<String>,
    pub protocol: Option<String>,
    /// `models` 槽位：`{"id","tags"}` 条目数组的 JSON 文本（验证层保证
    /// 形状，存储层不解释）。
    pub models: Option<String>,
    /// `model_map` 槽位：对象的 JSON 文本。
    pub model_map: Option<String>,
    #[column(default = "0")]
    pub deleted: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Item（provider 条目的 wire 形状）↔ 行 的转换字段表：列名与 JSON 键
/// 逐一相同（single-state-dir D3），字符串槽位直接对位。
const STRING_FIELDS: &[&str] = &[
    "name",
    "preset",
    "base_url_anthropic",
    "base_url_openai_chat",
    "base_url_openai_responses",
    "api_key",
    "api_key_env",
    "default_model",
    "protocol",
];

/// JSON 槽位（数组/对象以 JSON 文本进列）。
const JSON_FIELDS: &[&str] = &["models", "model_map"];

impl ProviderRow {
    /// Item → 行（写入路径）。键 = provider 实例名。
    ///
    /// null 与缺失同义（归一为 `None`）：条目验证层本就把 `models: null`
    /// 归一为移除；字符串槽位的显式 null 在存储后同样读回为缺失——线上
    /// 语义等价（缺字段 = 未配置）。
    pub fn from_item(id: &str, item: &serde_json::Map<String, Value>) -> Self {
        let now = sebas_domain::prim::now_unix();
        let mut row = Self {
            id: id.to_string(),
            name: None,
            preset: None,
            base_url_anthropic: None,
            base_url_openai_chat: None,
            base_url_openai_responses: None,
            api_key: None,
            api_key_env: None,
            default_model: None,
            protocol: None,
            models: None,
            model_map: None,
            deleted: 0,
            created_at: now,
            updated_at: now,
        };
        for field in STRING_FIELDS {
            if let Some(v) = item.get(*field).and_then(Value::as_str) {
                let slot = match *field {
                    "name" => &mut row.name,
                    "preset" => &mut row.preset,
                    "base_url_anthropic" => &mut row.base_url_anthropic,
                    "base_url_openai_chat" => &mut row.base_url_openai_chat,
                    "base_url_openai_responses" => &mut row.base_url_openai_responses,
                    "api_key" => &mut row.api_key,
                    "api_key_env" => &mut row.api_key_env,
                    "default_model" => &mut row.default_model,
                    "protocol" => &mut row.protocol,
                    _ => unreachable!("STRING_FIELDS 与槽位对位齐全"),
                };
                *slot = Some(v.to_string());
            }
        }
        for field in JSON_FIELDS {
            match item.get(*field) {
                Some(v) if !v.is_null() => {
                    let text = serde_json::to_string(v).unwrap_or_else(|_| {
                        unreachable!("serde_json::Value 序列化不会失败")
                    });
                    match *field {
                        "models" => row.models = Some(text),
                        "model_map" => row.model_map = Some(text),
                        _ => unreachable!("JSON_FIELDS 与槽位对位齐全"),
                    }
                }
                _ => {}
            }
        }
        row
    }

    /// 行 → Item（读取路径）。`None` 列不产生键——与「缺字段 = 未配置」
    /// 的线上语义一致。
    pub fn to_item(&self) -> serde_json::Map<String, Value> {
        let mut item = serde_json::Map::new();
        for field in STRING_FIELDS {
            let value = match *field {
                "name" => self.name.as_deref(),
                "preset" => self.preset.as_deref(),
                "base_url_anthropic" => self.base_url_anthropic.as_deref(),
                "base_url_openai_chat" => self.base_url_openai_chat.as_deref(),
                "base_url_openai_responses" => self.base_url_openai_responses.as_deref(),
                "api_key" => self.api_key.as_deref(),
                "api_key_env" => self.api_key_env.as_deref(),
                "default_model" => self.default_model.as_deref(),
                "protocol" => self.protocol.as_deref(),
                _ => unreachable!("STRING_FIELDS 与槽位对位齐全"),
            };
            if let Some(v) = value {
                item.insert((*field).to_string(), Value::String(v.to_string()));
            }
        }
        for field in JSON_FIELDS {
            let text = match *field {
                "models" => self.models.as_deref(),
                "model_map" => self.model_map.as_deref(),
                _ => unreachable!("JSON_FIELDS 与槽位对位齐全"),
            };
            if let Some(text) = text
                && let Ok(v) = serde_json::from_str::<Value>(text)
            {
                item.insert((*field).to_string(), v);
            }
        }
        item
    }
}

/// model_aliases 行（主键 alias；`provider` 列带 REFERENCES providers(id)
/// 外键——清空顺序必须先 aliases 后 providers）。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, SchemaColumns, ActiveRecord)]
#[active_record(table = "model_aliases")]
#[active_record(pk = "alias")]
pub struct ModelAliasRow {
    pub alias: String,
    pub provider: String,
    pub upstream_model: Option<String>,
    pub created_at: i64,
}

/// 按 provider 查别名（按非键列条件的手写查询——按 spec 返回 struct 实例）。
pub fn aliases_for_provider(
    conn: &mut Connection,
    provider: &str,
) -> Result<Vec<ModelAliasRow>, String> {
    use sebas_db::record::Record;
    let mut stmt = conn
        .prepare("SELECT alias, provider, upstream_model, created_at FROM model_aliases WHERE provider = ?1 ORDER BY alias")
        .map_err(|e| format!("准备 model_aliases 查询失败: {e}"))?;
    let rows = stmt
        .query_map(sebas_db::rusqlite::params![provider], ModelAliasRow::from_row)
        .map_err(|e| format!("查询 model_aliases 失败: {e}"))?;
    let mut aliases = Vec::new();
    for row in rows {
        aliases.push(row.map_err(|e| format!("读取 model_alias 行失败: {e}"))?);
    }
    Ok(aliases)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(entries: &[(&str, Value)]) -> serde_json::Map<String, Value> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    /// single-state-dir 3.2 验收：channel 上的 provider JSON 形状经 serde
    /// 往返与改造前一致——全部字符串槽位 + 两个 JSON 槽位的条目。
    #[test]
    fn item_row_item_round_trip_preserves_wire_shape() {
        let original = item(&[
            ("name", Value::String("my-deepseek".into())),
            ("preset", Value::String("deepseek".into())),
            (
                "base_url_anthropic",
                Value::String("https://api.anthropic.com".into()),
            ),
            (
                "base_url_openai_chat",
                Value::String("https://api.deepseek.com/chat".into()),
            ),
            (
                "base_url_openai_responses",
                Value::String("https://api.deepseek.com/responses".into()),
            ),
            ("api_key", Value::String("sk-x".into())),
            ("api_key_env", Value::String("DEEPSEEK_API_KEY".into())),
            ("default_model", Value::String("deepseek-chat".into())),
            ("protocol", Value::String("anthropic".into())),
            (
                "models",
                serde_json::json!([{"id": "deepseek-chat", "tags": []}]),
            ),
            ("model_map", serde_json::json!({"deepseek-chat": "d-chat"})),
        ]);

        let row = ProviderRow::from_item("my-deepseek", &original);
        assert_eq!(row.id, "my-deepseek");
        assert_eq!(row.preset.as_deref(), Some("deepseek"));
        assert_eq!(
            row.base_url_openai_chat.as_deref(),
            Some("https://api.deepseek.com/chat")
        );
        let back = row.to_item();
        assert_eq!(back, original, "wire 形状经 serde 往返逐字段一致");
    }

    /// 缺失槽位不产生键；`models`/`model_map` 的显式 null 归一为缺失
    /// （验证层在写路径同样移除 null models——两侧语义一致）。
    #[test]
    fn absent_and_null_fields_stay_absent_on_wire() {
        let partial = item(&[("preset", Value::String("anthropic".into()))]);
        let row = ProviderRow::from_item("anthropic", &partial);
        let back = row.to_item();
        assert_eq!(back, partial);
        assert!(!back.contains_key("name"));
        assert!(!back.contains_key("api_key"));

        let nulled = item(&[
            ("preset", Value::String("anthropic".into())),
            ("api_key", Value::Null),
            ("models", Value::Null),
        ]);
        let row = ProviderRow::from_item("anthropic", &nulled);
        let back = row.to_item();
        assert_eq!(
            back,
            item(&[("preset", Value::String("anthropic".into()))]),
            "null 归一为缺失（缺字段 = 未配置）"
        );
    }

    /// 存储契约：Item = Map 不再直接进存储路径——行才是事实源；
    /// ActiveRecord 的 upsert SQL 以新列清单生成（golden 形状钉在根
    /// crate 的 repo 测试里，这里钉列清单）。
    #[test]
    fn schema_columns_are_the_typed_provider_columns() {
        use sebas_db::record::Record;
        assert_eq!(
            ProviderRow::COLUMNS,
            &[
                "id",
                "name",
                "preset",
                "base_url_anthropic",
                "base_url_openai_chat",
                "base_url_openai_responses",
                "api_key",
                "api_key_env",
                "default_model",
                "protocol",
                "models",
                "model_map",
                "deleted",
                "created_at",
                "updated_at",
            ]
        );
        assert_eq!(ProviderRow::PK_COLUMNS, &["id"]);
        assert_eq!(ProviderRow::TABLE, "providers");
    }

    /// 行的 CRUD 往返（save → find → 全等）：JSON 槽位以文本进列、按文本
    /// 出列，行全等（含逐字段语义）。
    #[test]
    fn typed_row_round_trips_through_sqlite() {
        use sebas_db::schema::{TableSchema, open_and_sync};
        let dir = tempfile::tempdir().unwrap();
        static TABLES: &[TableSchema] = &[TableSchema {
            name: "providers",
            create_ddl: "CREATE TABLE providers (
                id          TEXT PRIMARY KEY,
                name        TEXT,
                preset      TEXT,
                base_url_anthropic        TEXT,
                base_url_openai_chat      TEXT,
                base_url_openai_responses TEXT,
                api_key     TEXT,
                api_key_env TEXT,
                default_model TEXT,
                protocol    TEXT,
                models      TEXT,
                model_map   TEXT,
                deleted     INTEGER NOT NULL DEFAULT 0,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL
            );
            CREATE INDEX idx_providers_deleted ON providers(deleted);",
            columns: ProviderRow::schema_columns(),
        }];
        let conn = open_and_sync(&dir.path().join("p.db"), TABLES).unwrap().0;

        let original = item(&[
            ("name", Value::String("my-deepseek".into())),
            ("preset", Value::String("deepseek".into())),
            ("api_key", Value::String("sk-x".into())),
            (
                "models",
                serde_json::json!([{"id": "deepseek-chat", "tags": ["vision"]}]),
            ),
            ("model_map", serde_json::json!({"a": "b"})),
        ]);
        let row = ProviderRow::from_item("my-deepseek", &original);
        row.save(&conn).unwrap();
        let found = ProviderRow::find(&conn, "my-deepseek").unwrap().unwrap();
        assert_eq!(found, row, "行经 SQLite 存取逐字段全等");
        assert_eq!(found.to_item(), original, "读回的行还原出原 wire 形状");
    }
}
