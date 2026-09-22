//! `SettingRow` — settings 表（KV）的 ActiveRecord struct。
//!
//! settings 的 value 暂为 JSON blob 列（形状不变，extract-sebas-db 不重塑）；
//! 读写经本 struct 的生成的 CRUD，不再手写逐表 SQL。

use sebas_schema_derive::{ActiveRecord, SchemaColumns};

/// settings 行 (key-value)，表结构的单一事实源。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, SchemaColumns, ActiveRecord)]
#[active_record(table = "settings")]
#[active_record(pk = "key")]
pub struct SettingRow {
    pub key: String,
    pub value: String,
}
