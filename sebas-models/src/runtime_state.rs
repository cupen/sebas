//! runtime 状态（mode + default_selection）与 legacy defaults 一次性导入的
//! 域查询（repo.rs 原位搬迁，make-core-own-provider-data D3/1.4 语义不变）。
//!
//! runtime 状态整体作为 `settings` 表 `runtime_state` 键下的一个 JSON 行存
//! 储——通过 [`SettingRow`]（ActiveRecord）读写，本模块只定义 JSON 形状
//! （[`RuntimeStateRow`]) 与域级语义。

use crate::setting::SettingRow;
use sebas_db::rusqlite::Connection;
use sebas_domain::provider::{DefaultSelection, ProviderMode};
use serde::{Deserialize, Serialize};

/// 运行时状态行 (mode + default_selection) 的 JSON 形状。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeStateRow {
    #[serde(default)]
    pub mode: ProviderMode,
    #[serde(default)]
    pub default_selection: Option<DefaultSelection>,
}

/// settings 表里 runtime 状态行的键。
pub const RUNTIME_STATE_KEY: &str = "runtime_state";

/// legacy defaults 一次性导入的标记键（在场即不再读 legacy defaults.json）。
pub const DEFAULTS_IMPORTED_KEY: &str = "defaults_imported";

/// 从 DB 加载 runtime 状态 (mode + default_selection)。行缺失/损坏时取
/// 缺省值（与既有行为一致：损坏 warn 后回落 default）。
pub fn load_runtime_state(conn: &mut Connection) -> (ProviderMode, Option<DefaultSelection>) {
    let raw: Option<String> =
        SettingRow::find(conn, RUNTIME_STATE_KEY)
            .ok()
            .flatten()
            .map(|row| row.value);

    match raw {
        Some(raw) => match serde_json::from_str::<RuntimeStateRow>(&raw) {
            Ok(row) => (row.mode, row.default_selection),
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse runtime_state, using defaults");
                (ProviderMode::default(), None)
            }
        },
        None => (ProviderMode::default(), None),
    }
}

/// 保存 runtime 状态（settings 表 `runtime_state` 行的 upsert）。
/// 取 `&Connection`：`&mut Connection` 自动转借，事务句柄 `&Transaction`
/// 经 Deref 也可直接传入。
pub fn save_runtime_state(conn: &Connection, state: &RuntimeStateRow) -> Result<(), String> {
    let value = serde_json::to_string(state).map_err(|e| format!("序列化 runtime state 失败: {e}"))?;
    SettingRow {
        key: RUNTIME_STATE_KEY.to_string(),
        value,
    }
    .save(conn)
    .map_err(|e| format!("写入 runtime state 失败: {e}"))
}

/// 导入标记是否在场（`settings` 表 `defaults_imported` 行）。在场即不再读
/// legacy defaults.json。
pub fn defaults_import_done(conn: &mut Connection) -> Result<bool, String> {
    let done = SettingRow::find(conn, DEFAULTS_IMPORTED_KEY)
        .map_err(|e| format!("读取 defaults_imported 标记失败: {e}"))?;
    Ok(done.is_some())
}

/// 标记导入阶段完成（无值可导也落标记——阶段一次性，不每次启动重放）。
pub fn mark_defaults_imported(conn: &mut Connection) -> Result<(), String> {
    SettingRow {
        key: DEFAULTS_IMPORTED_KEY.to_string(),
        value: "1".to_string(),
    }
    .save(conn)
    .map_err(|e| format!("写入 defaults_imported 标记失败: {e}"))
}

/// 导入默认值 + 落标记，**同一事务**完成（与 provider 数据同库同事务的
/// D3 语义）。已导入过 → Ok(false)，不覆盖库里的现值（用户后来的选择
/// 优先于 legacy 文件）。
pub fn import_defaults_once(
    conn: &mut Connection,
    selection: DefaultSelection,
) -> Result<bool, String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("导入事务开始失败: {e}"))?;
    let done = SettingRow::find(&tx, DEFAULTS_IMPORTED_KEY)
        .map_err(|e| format!("读取 defaults_imported 标记失败: {e}"))?;
    if done.is_some() {
        return Ok(false);
    }
    // RMW runtime_state：只改 default_selection，mode 原样保留。
    let existing = SettingRow::find(&tx, RUNTIME_STATE_KEY)
        .map_err(|e| format!("读取 runtime state 失败: {e}"))?;
    let mut row: RuntimeStateRow = existing
        .and_then(|r| serde_json::from_str(&r.value).ok())
        .unwrap_or_default();
    row.default_selection = Some(selection);
    let runtime_json =
        serde_json::to_string(&row).map_err(|e| format!("序列化 runtime state 失败: {e}"))?;
    SettingRow {
        key: RUNTIME_STATE_KEY.to_string(),
        value: runtime_json,
    }
    .save(&tx)
    .map_err(|e| format!("写入 runtime state 失败: {e}"))?;
    SettingRow {
        key: DEFAULTS_IMPORTED_KEY.to_string(),
        value: "1".to_string(),
    }
    .save(&tx)
    .map_err(|e| format!("写入 defaults_imported 标记失败: {e}"))?;
    tx.commit().map_err(|e| format!("导入事务提交失败: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sebas_db::schema::TableSchema;

    /// 中性测试表（小写 DDL——域 DDL 留在根注册表，这里只建 settings 形状
    /// 的中立替身）。
    static KV_TABLES: &[TableSchema] = &[TableSchema {
        name: "settings",
        create_table_ddl: "create table settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        index_ddls: &[],
        columns: crate::setting::SettingRow::schema_columns(),
    }];

    fn conn() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let (conn, _) = sebas_db::schema::open_and_sync(&dir.path().join("t.db"), KV_TABLES)
            .unwrap();
        (dir, conn)
    }

    #[test]
    fn runtime_state_round_trips() {
        let (_dir, mut conn) = conn();
        let (mode, sel) = load_runtime_state(&mut conn);
        assert_eq!(mode, ProviderMode::default());
        assert_eq!(sel, None);

        save_runtime_state(
            &mut conn,
            &RuntimeStateRow {
                mode: ProviderMode::Router,
                default_selection: Some(DefaultSelection::with_model("p", "m")),
            },
        )
        .unwrap();
        let (mode, sel) = load_runtime_state(&mut conn);
        assert_eq!(mode, ProviderMode::Router);
        assert_eq!(sel, Some(DefaultSelection::with_model("p", "m")));
    }

    #[test]
    fn defaults_import_once_is_idempotent() {
        let (_dir, mut conn) = conn();
        assert!(!defaults_import_done(&mut conn).unwrap());
        assert!(
            import_defaults_once(&mut conn, DefaultSelection::new("legacy")).unwrap(),
            "首次导入应生效"
        );
        assert!(defaults_import_done(&mut conn).unwrap());
        // 已导入 → 不覆盖（用户选择优先）。
        save_runtime_state(
            &mut conn,
            &RuntimeStateRow {
                mode: ProviderMode::Router,
                default_selection: Some(DefaultSelection::new("user-pick")),
            },
        )
        .unwrap();
        assert!(!import_defaults_once(&mut conn, DefaultSelection::new("legacy2")).unwrap());
        let (_, sel) = load_runtime_state(&mut conn);
        assert_eq!(sel, Some(DefaultSelection::new("user-pick")));
    }
}
