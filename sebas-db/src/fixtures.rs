//! 测试专用中性夹具（仅 `#[cfg(test)]` 编译）。
//!
//! 表名/列名全部是中性词（`alpha` / `kv`），DDL 用小写关键词——**不得**
//! 出现任何域表名或域类型（sebas-db 的域无关纪律，见根 crate
//! `tests/persistence_runtime_test.rs` 的机械断言）。

use crate::schema::{SchemaColumn, TableSchema};

/// alpha 表的派生列清单（与真实模型同构：主键列无非空默认、可空列
/// `not_null: false`、带默认列 `default: Some("0")`、无改名来源）。
pub(crate) static ALPHA_COLUMNS: &[SchemaColumn] = &[
    SchemaColumn {
        name: "id",
        affinity: "TEXT",
        default: None,
        not_null: true,
        rename_from: None,
    },
    SchemaColumn {
        name: "name",
        affinity: "TEXT",
        default: None,
        not_null: true,
        rename_from: None,
    },
    SchemaColumn {
        name: "note",
        affinity: "TEXT",
        default: None,
        not_null: false,
        rename_from: None,
    },
    SchemaColumn {
        name: "score",
        affinity: "INTEGER",
        default: Some("0"),
        not_null: true,
        rename_from: None,
    },
];

/// 同步算法的测试注册表：单表 `alpha`（主键 id，含可空列与带默认列）。
pub(crate) static TEST_TABLES: &[TableSchema] = &[TableSchema {
    name: "alpha",
    create_table_ddl: "create table alpha (
        id    TEXT PRIMARY KEY,
        name  TEXT NOT NULL,
        note  TEXT,
        score INTEGER NOT NULL DEFAULT 0
    );",
    index_ddls: &[],
    columns: ALPHA_COLUMNS,
}];

/// record / writer 测试用的单表 `kv`（单列主键）。
pub(crate) static KV_TABLES: &[TableSchema] = &[TableSchema {
    name: "kv",
    create_table_ddl: "create table kv (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL,
        flag  INTEGER NOT NULL DEFAULT 0
    );",
    index_ddls: &[],
    columns: &[
        SchemaColumn {
            name: "key",
            affinity: "TEXT",
            default: None,
            not_null: true,
            rename_from: None,
        },
        SchemaColumn {
            name: "value",
            affinity: "TEXT",
            default: None,
            not_null: true,
            rename_from: None,
        },
        SchemaColumn {
            name: "flag",
            affinity: "INTEGER",
            default: Some("0"),
            not_null: true,
            rename_from: None,
        },
    ],
}];

/// `kv` 表的中性测试行（手写 `Record` impl——本 crate 内不能用自家
/// derive：生成路径 `::sebas_db` 在 crate 内不解析）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KvRow {
    pub key: String,
    pub value: String,
    pub flag: bool,
}

impl crate::record::Record for KvRow {
    const TABLE: &'static str = "kv";
    const PK_COLUMNS: &'static [&'static str] = &["key"];
    const COLUMNS: &'static [&'static str] = &["key", "value", "flag"];

    fn to_params(&self) -> Vec<&dyn rusqlite::ToSql> {
        vec![
            &self.key as &dyn rusqlite::ToSql,
            &self.value as &dyn rusqlite::ToSql,
            &self.flag as &dyn rusqlite::ToSql,
        ]
    }

    fn pk_params(&self) -> Vec<&dyn rusqlite::ToSql> {
        vec![&self.key as &dyn rusqlite::ToSql]
    }

    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            key: row.get(0)?,
            value: row.get(1)?,
            flag: row.get(2)?,
        })
    }
}
