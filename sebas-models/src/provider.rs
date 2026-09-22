//! `ProviderRow` / `ModelAliasRow` — providers 与 model_aliases 表的
//! ActiveRecord struct + 按非键列的域查询。
//!
//! `ProviderRow.config` 暂为 JSON blob 列（providers 的表形状重塑属
//! `single-state-dir`）；无类型 `Map<String, Value>` 作为存储契约的地位已
//! 终结——读写一律经本 struct（design D3b）。

use sebas_db::rusqlite::Connection;
use sebas_schema_derive::{ActiveRecord, SchemaColumns};

/// providers 行。PRIMARY KEY 等约束只在根 crate 注册表的 DDL 里表达。
/// 读取路径可能只取部分列，其余字段仅为 schema 声明存在, 故整体
/// allow(dead_code)。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, SchemaColumns, ActiveRecord)]
#[active_record(table = "providers")]
#[active_record(pk = "id")]
pub struct ProviderRow {
    pub id: String,
    pub config: String,
    #[column(default = "0")]
    pub deleted: i64,
    pub created_at: i64,
    pub updated_at: i64,
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
