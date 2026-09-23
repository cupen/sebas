//! `ProjectRow` — projects 表的 ActiveRecord struct + 项目域查询。
//!
//! struct 即表结构的列事实源（列名/类型/默认值/可空性）；约束（PRIMARY
//! KEY `path` / UNIQUE `id`）与索引只在根 crate 注册表的手写 DDL 里表达。
//! 非标准查询（排序、按非键列 UPDATE）保留手写 SQL，但一律返回 struct
//! 实例或标量。

use sebas_db::rusqlite::Connection;
use sebas_schema_derive::{ActiveRecord, SchemaColumns};

/// projects 表行（JSON 兼容形状, 与 `sebas_domain::project::ProjectEntry`
/// 对应）。列顺序即建表事实（sqlite-auto-schema-sync）。
#[derive(
    Debug, Clone, PartialEq, Eq, SchemaColumns, ActiveRecord, serde::Serialize,
    serde::Deserialize,
)]
#[active_record(table = "projects")]
#[active_record(pk = "path")]
pub struct ProjectRow {
    /// 稳定项目 id（`proj-<12hex>`；workbench-agent-wire-fix 2.4）。
    /// 迁移 2 之前的行读取时为 None，由应用层按 path 回填。
    pub id: Option<String>,
    pub path: String,
    pub name: String,
    pub default_agent: Option<String>,
    pub branch: Option<String>,
    #[column(default = "0")]
    pub branch_at: i64,
    pub added_at: i64,
    #[column(default = "0")]
    pub sort_order: i64,
}

impl ProjectRow {
    /// ProjectRow → [`sebas_domain::project::ProjectEntry`]（add-domain-layer
    /// 3.5，design D6）：两个合法形状之间的**命名显式转换**。
    ///
    /// 有损方向（逐项钉在测试里）：DB 形状今天没有节点维度，`node_id`
    /// 一律回填 `local`（远端条目的家是文件注册表）；`id` 为 NULL（迁移 2
    /// 之前的行）回退空串。
    pub fn to_entry(&self) -> sebas_domain::project::ProjectEntry {
        sebas_domain::project::ProjectEntry {
            id: self.id.clone().unwrap_or_default(),
            path: self.path.clone(),
            name: self.name.clone(),
            added_at: u64::try_from(self.added_at).unwrap_or(0),
            default_agent: self.default_agent.clone(),
            branch: self.branch.clone(),
            branch_at: u64::try_from(self.branch_at).unwrap_or(0),
            node_id: sebas_domain::project::LOCAL_NODE_ID.to_string(),
        }
    }
}

/// [`sebas_domain::project::ProjectEntry`] → `ProjectRow`（add-domain-layer
/// 3.5，design D6）：反方向转换。有损方向：`node_id` 无列可载，随 DB 形状
/// 消失；`sort_order` 不属注册表形状，落 DB 缺省 0。
impl From<&sebas_domain::project::ProjectEntry> for ProjectRow {
    fn from(e: &sebas_domain::project::ProjectEntry) -> Self {
        Self {
            id: Some(e.id.clone()),
            path: e.path.clone(),
            name: e.name.clone(),
            default_agent: e.default_agent.clone(),
            branch: e.branch.clone(),
            branch_at: i64::try_from(e.branch_at).unwrap_or(0),
            added_at: i64::try_from(e.added_at).unwrap_or(0),
            sort_order: 0,
        }
    }
}

// ---- 非标准查询（返回 struct 实例；标准 CRUD 走生成的 save/find/delete）----

/// 加载所有项目（注册表列表的展示顺序：sort_order, added_at——非标准排序
/// 查询，保留手写 SELECT，列顺序与 `COLUMNS` 一致以复用 `from_row`）。
pub fn load_projects(conn: &mut Connection) -> Result<Vec<ProjectRow>, String> {
    use sebas_db::record::Record;
    let sql = format!(
        "SELECT {} FROM projects ORDER BY sort_order, added_at",
        <ProjectRow as Record>::COLUMNS.join(", ")
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("准备 projects 查询失败: {e}"))?;
    let rows = stmt
        .query_map([], ProjectRow::from_row)
        .map_err(|e| format!("查询 projects 失败: {e}"))?;
    let mut projects = Vec::new();
    for row in rows {
        projects.push(row.map_err(|e| format!("读取 project 行失败: {e}"))?);
    }
    Ok(projects)
}

/// 保存所有项目 (全量替换, 单事务)。
pub fn save_projects(conn: &mut Connection, projects: &[ProjectRow]) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("保存 projects 事务开始失败: {e}"))?;

    tx.execute("DELETE FROM projects", [])
        .map_err(|e| format!("清空 projects 表失败: {e}"))?;

    for p in projects {
        p.save(&tx)
            .map_err(|e| format!("写入 project {} 失败: {e}", p.path))?;
    }

    tx.commit()
        .map_err(|e| format!("保存 projects 事务提交失败: {e}"))?;
    Ok(())
}

/// 添加一个项目（已存在同 path 时静默跳过——注册表幂等语义）。
///
/// single-state-dir 3.6：标准 CRUD 经生成的 `find`/`save` 组合——单写线程
/// 串行执行下 find→save 与 `ON CONFLICT DO NOTHING` 等价（不存在并发插队
/// 窗口），不再保留手写 INSERT。
pub fn add_project(
    conn: &mut Connection,
    path: &str,
    name: &str,
    added_at: i64,
) -> Result<(), String> {
    if ProjectRow::find(conn, path)
        .map_err(|e| format!("查询项目 {path} 失败: {e}"))?
        .is_some()
    {
        return Ok(());
    }
    ProjectRow {
        id: None,
        path: path.to_string(),
        name: name.to_string(),
        default_agent: None,
        branch: None,
        branch_at: 0,
        added_at,
        sort_order: 0,
    }
    .save(conn)
    .map_err(|e| format!("添加项目 {path} 失败: {e}"))?;
    Ok(())
}

/// 记录项目级默认 agent（workbench-agent-wire-fix 2.6），按稳定 id 定位
/// （id 是 UNIQUE 索引而非主键——按非键列条件的非标准查询）。
pub fn set_project_default_agent(
    conn: &mut Connection,
    id: &str,
    agent: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE projects SET default_agent = ?2 WHERE id = ?1",
        sebas_db::rusqlite::params![id, agent],
    )
    .map_err(|e| format!("更新项目 {id} 默认 agent 失败: {e}"))?;
    Ok(())
}

/// 删除项目（按主键 path）。返回是否确有行被删。
pub fn remove_project(conn: &mut Connection, path: &str) -> Result<bool, String> {
    ProjectRow::delete(conn, path).map_err(|e| format!("删除项目 {path} 失败: {e}"))
}

/// 更新项目分支信息（按主键 path 定位的部分更新——非标准，保留手写 UPDATE）。
pub fn update_project_branch(
    conn: &mut Connection,
    path: &str,
    branch: Option<&str>,
    branch_at: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE projects SET branch = ?1, branch_at = ?2 WHERE path = ?3",
        sebas_db::rusqlite::params![branch, branch_at, path],
    )
    .map_err(|e| format!("更新项目分支 {path} 失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ProjectRow;
    use sebas_domain::project::{ProjectEntry, LOCAL_NODE_ID};

    fn row() -> ProjectRow {
        ProjectRow {
            id: Some("proj-abcdef123456".into()),
            path: "/data/work".into(),
            name: "work".into(),
            default_agent: Some("claude".into()),
            branch: Some("main".into()),
            branch_at: 1_700_000_100,
            added_at: 1_700_000_000,
            sort_order: 3,
        }
    }

    /// DB 形状钉：列顺序/列名即建表事实（sqlite-auto-schema-sync），序列化
    /// 形状加字段即测试失败。
    #[test]
    fn project_row_serialized_shape_is_pinned() {
        let v = serde_json::to_value(row()).unwrap();
        let obj = v.as_object().unwrap();
        let keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "id", "path", "name", "default_agent", "branch", "branch_at", "added_at",
                "sort_order"
            ]
        );
        assert_eq!(obj["id"], "proj-abcdef123456");
        assert_eq!(obj["sort_order"], 3);
    }

    /// 列元数据形状钉（4.5 基线）：列名/亲和/默认值/可空性逐列等于迁移前
    /// 根 crate 录制的基线。
    #[test]
    fn schema_columns_match_pre_migration_baseline() {
        use sebas_db::record::Record;
        use sebas_db::schema::SchemaColumn;
        assert_eq!(ProjectRow::TABLE, "projects");
        assert_eq!(ProjectRow::PK_COLUMNS, &["path"]);
        let baseline: &[SchemaColumn] = &[
            SchemaColumn { name: "id", affinity: "TEXT", default: None, not_null: false },
            SchemaColumn { name: "path", affinity: "TEXT", default: None, not_null: true },
            SchemaColumn { name: "name", affinity: "TEXT", default: None, not_null: true },
            SchemaColumn { name: "default_agent", affinity: "TEXT", default: None, not_null: false },
            SchemaColumn { name: "branch", affinity: "TEXT", default: None, not_null: false },
            SchemaColumn { name: "branch_at", affinity: "INTEGER", default: Some("0"), not_null: true },
            SchemaColumn { name: "added_at", affinity: "INTEGER", default: None, not_null: true },
            SchemaColumn { name: "sort_order", affinity: "INTEGER", default: Some("0"), not_null: true },
        ];
        assert_eq!(ProjectRow::schema_columns(), baseline);
        assert_eq!(ProjectRow::COLUMNS, baseline.iter().map(|c| c.name).collect::<Vec<_>>().as_slice());
    }

    /// Row → Entry：共享字段保值；node_id 回填 local；NULL id 回退空串。
    #[test]
    fn row_to_entry_preserves_shared_fields_and_backfills_node() {
        let e = row().to_entry();
        assert_eq!(e.id, "proj-abcdef123456");
        assert_eq!(e.path, "/data/work");
        assert_eq!(e.name, "work");
        assert_eq!(e.default_agent.as_deref(), Some("claude"));
        assert_eq!(e.branch.as_deref(), Some("main"));
        assert_eq!(e.branch_at, 1_700_000_100);
        assert_eq!(e.added_at, 1_700_000_000);
        assert_eq!(e.node_id, LOCAL_NODE_ID);

        let mut legacy = row();
        legacy.id = None;
        assert_eq!(legacy.to_entry().id, "", "迁移 2 之前的 NULL id 回退空串");
    }

    /// Entry → Row → Entry 往返：注册表形状的字段全部保值（sort_order 是
    /// Row 独有、node_id 是 Entry 独有——两侧有损性由本测试钉住）。
    #[test]
    fn entry_row_round_trip_preserves_registry_shape() {
        let entry = ProjectEntry {
            id: "proj-1".into(),
            path: "/tmp/p".into(),
            name: "p".into(),
            added_at: 7,
            default_agent: None,
            branch: None,
            branch_at: 0,
            node_id: "node-b".into(),
        };
        let row = ProjectRow::from(&entry);
        assert_eq!(row.sort_order, 0, "Entry 无排序语义，落 DB 缺省 0");
        let back = row.to_entry();
        assert_eq!(back.id, "proj-1");
        assert_eq!(back.added_at, 7);
        // node_id 有损：DB 形状今天没有节点列。
        assert_eq!(back.node_id, LOCAL_NODE_ID);
        assert_ne!(back.node_id, entry.node_id);
    }
}
