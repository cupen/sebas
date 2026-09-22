//! 泛型 `Record` trait 与标准 CRUD 的 SQL 构建器（extract-sebas-db 3.1，D3b）。
//!
//! **runtime 只认识 trait，不认识任何域类型**（spec「The execution model
//! carries no domain knowledge」）：表名、主键列、列清单与行映射全部由实现
//! 提供——生产实现由 `#[derive(ActiveRecord)]`（`sebas-schema-derive`）在
//! struct 定义处生成，测试实现手写（见 `fixtures`）。
//!
//! 固有方法（`row.save(conn)`）生成在使用方 crate（impl 与 struct 必须
//! 同 crate），因此这里只提供 trait 与其上的自由函数；`writer::StateHandle`
//! 的类型化门面经同一个 trait 工作，同样不知道任何域类型。

use rusqlite::Connection;

/// 一个表对应一个 struct、一行对应一个实例的映射声明。
///
/// 实现方承诺：
/// - [`Self::COLUMNS`] 的顺序 = [`Self::to_params`] 的绑定顺序 =
///   [`Self::from_row`] 的取列顺序（生成的 upsert / 查询按此拼 SQL）；
/// - [`Self::PK_COLUMNS`] 按表的 DDL 主键顺序排列（复合主键按全部键列）。
pub trait Record: Sized {
    /// 表名。域无关：runtime 不解释它的内容。
    const TABLE: &'static str;
    /// 主键列名（按 DDL 顺序；单列主键长度为 1）。
    const PK_COLUMNS: &'static [&'static str];
    /// 全部列名（与字段顺序一致）。
    const COLUMNS: &'static [&'static str];

    /// 行 → 全列绑定参数（顺序与 [`Self::COLUMNS`] 一致）。
    fn to_params(&self) -> Vec<&dyn rusqlite::ToSql>;

    /// 行 → 主键绑定参数（顺序与 [`Self::PK_COLUMNS`] 一致）。
    fn pk_params(&self) -> Vec<&dyn rusqlite::ToSql>;

    /// 行 ← 列映射（按 [`Self::COLUMNS`] 顺序取列）。
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self>;
}

/// 生成的标准 upsert SQL：`INSERT … ON CONFLICT(pk) DO UPDATE SET …`。
///
/// 注意：全列都是主键的表（无非键列）没有可更新的 SET 目标，生成的
/// `DO UPDATE SET` 为空——这类表请走手写 SQL（工作区内不存在此形态）。
pub fn upsert_sql<R: Record>() -> String {
    let cols = R::COLUMNS.join(", ");
    let placeholders: Vec<String> = (1..=R::COLUMNS.len()).map(|i| format!("?{i}")).collect();
    let non_pk: Vec<&str> = R::COLUMNS
        .iter()
        .copied()
        .filter(|c| !R::PK_COLUMNS.contains(c))
        .collect();
    let sets: Vec<String> = non_pk.iter().map(|c| format!("{c} = excluded.{c}")).collect();
    format!(
        "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT({}) DO UPDATE SET {}",
        R::TABLE,
        cols,
        placeholders.join(", "),
        R::PK_COLUMNS.join(", "),
        sets.join(", ")
    )
}

/// 生成的标准查询 SQL（`all` / `find` 共用的 SELECT 形状）。
pub fn select_sql<R: Record>() -> String {
    format!("SELECT {} FROM {}", R::COLUMNS.join(", "), R::TABLE)
}

/// 生成的标准删除 SQL（单列主键形态）。
pub fn delete_sql<R: Record>() -> String {
    format!(
        "DELETE FROM {} WHERE {} = ?1",
        R::TABLE, R::PK_COLUMNS[0]
    )
}

/// 保存一行：按主键 upsert（存在则更新非键列，不存在则插入）。
pub fn save<R: Record>(conn: &Connection, record: &R) -> rusqlite::Result<()> {
    let sql = upsert_sql::<R>();
    conn.execute(&sql, record.to_params().as_slice())?;
    Ok(())
}

/// 按单列主键取一行；不存在返回 `None`。
pub fn find<R: Record, P: rusqlite::ToSql>(
    conn: &Connection,
    pk: P,
) -> rusqlite::Result<Option<R>> {
    let sql = format!(
        "{} WHERE {} = ?1",
        select_sql::<R>(),
        R::PK_COLUMNS[0]
    );
    match conn.query_row(&sql, [pk], R::from_row) {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 按全部主键列取一行（复合主键形态）。注意：键列含 NULL 时 SQL 等值比较
/// 不命中（`NULL = NULL` 非真）——与手写 SQL 的语义一致。
pub fn find_by<R: Record>(conn: &Connection, pks: &[&dyn rusqlite::ToSql]) -> rusqlite::Result<Option<R>> {
    let sql = format!("{} WHERE {}", select_sql::<R>(), pk_equals_sql::<R>());
    match conn.query_row(&sql, pks, R::from_row) {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 取全表行。顺序未定义（无 ORDER BY）——需要顺序的查询是"非标准查询"，
/// 由使用方手写并返回 struct 实例。
pub fn all<R: Record>(conn: &Connection) -> rusqlite::Result<Vec<R>> {
    let sql = select_sql::<R>();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], R::from_row)?;
    rows.collect()
}

/// 按单列主键删一行。返回是否确有行被删。
pub fn delete<R: Record, P: rusqlite::ToSql>(conn: &Connection, pk: P) -> rusqlite::Result<bool> {
    let sql = delete_sql::<R>();
    Ok(conn.execute(&sql, [pk])? > 0)
}

/// 按全部主键列删一行（复合主键形态）。
pub fn delete_by<R: Record>(conn: &Connection, pks: &[&dyn rusqlite::ToSql]) -> rusqlite::Result<bool> {
    let sql = format!("DELETE FROM {} WHERE {}", R::TABLE, pk_equals_sql::<R>());
    Ok(conn.execute(&sql, pks)? > 0)
}

/// `pk1 = ?1 AND pk2 = ?2 …`（按 [`Record::PK_COLUMNS`] 顺序）。
fn pk_equals_sql<R: Record>() -> String {
    R::PK_COLUMNS
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{c} = ?{}", i + 1))
        .collect::<Vec<_>>()
        .join(" AND ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn;
    use crate::fixtures::KvRow;
    use crate::schema::open_and_sync;
    use crate::fixtures;
    use tempfile::tempdir;

    fn kv_conn() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("kv.db");
        let (conn, _) = open_and_sync(&path, fixtures::KV_TABLES).unwrap();
        (dir, conn)
    }

    #[test]
    fn upsert_sql_shape_is_pinned() {
        assert_eq!(
            upsert_sql::<KvRow>(),
            "INSERT INTO kv (key, value, flag) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, flag = excluded.flag"
        );
        assert_eq!(select_sql::<KvRow>(), "SELECT key, value, flag FROM kv");
        assert_eq!(delete_sql::<KvRow>(), "DELETE FROM kv WHERE key = ?1");
    }

    /// trait 单测：upsert / 查 / 列 / 删四操作全走一遍（3.1 验收）。
    #[test]
    fn record_crud_round_trip() {
        let (_dir, conn) = kv_conn();

        // save（INSERT 分支）
        let row = KvRow { key: "k1".into(), value: "v1".into(), flag: true };
        save(&conn, &row).unwrap();

        // find 命中
        let found = find::<KvRow, _>(&conn, "k1").unwrap().expect("命中");
        assert_eq!(found, row);

        // find 未命中
        assert!(find::<KvRow, _>(&conn, "nope").unwrap().is_none());

        // all
        let row2 = KvRow { key: "k2".into(), value: "v2".into(), flag: false };
        save(&conn, &row2).unwrap();
        let mut all_rows = all::<KvRow>(&conn).unwrap();
        all_rows.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(all_rows, vec![row.clone(), row2]);

        // save（UPDATE 分支：同主键覆盖）
        let updated = KvRow { key: "k1".into(), value: "v1-changed".into(), flag: false };
        save(&conn, &updated).unwrap();
        let found = find::<KvRow, _>(&conn, "k1").unwrap().unwrap();
        assert_eq!(found, updated);

        // delete
        assert!(delete::<KvRow, _>(&conn, "k1").unwrap());
        assert!(!delete::<KvRow, _>(&conn, "k1").unwrap(), "再删同一行 = 无行受影响");
        assert!(find::<KvRow, _>(&conn, "k1").unwrap().is_none());
        assert_eq!(all::<KvRow>(&conn).unwrap().len(), 1);
    }
}
