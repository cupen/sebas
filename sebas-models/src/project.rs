//! `ProjectRow` — projects 表的 ActiveRecord struct + 项目域查询。
//!
//! **规范项目记录**（migrate-project-registry 2.1，design D2）：存储形状与
//! 线形状是同一个 struct——serde 序列化即 wire 形状（webui 经 channel 消费），
//! ActiveRecord 即持久化形状；`ProjectEntry` 是它的再导出别名。字段清单只
//! 在这里，两侧拼写由形状钉测试锁住（`node_id` 缺省 `local`、可选字段缺席
//! 不序列化）。
//!
//! struct 即表结构的列事实源（列名/类型/默认值/可空性）；约束（PRIMARY
//! KEY `path` / UNIQUE `id`）与索引只在根 crate 注册表的手写 DDL 里表达。
//! 非标准查询（排序、按非键列 UPDATE）保留手写 SQL，但一律返回 struct
//! 实例或标量。

use sebas_db::rusqlite::Connection;
use sebas_schema_derive::{ActiveRecord, SchemaColumns};

/// 本机节点的标识（旧数据与本地注册都用它）。
pub const LOCAL_NODE_ID: &str = "local";

/// 项目的稳定 id：`proj-<12hex>`，由 `(节点, 规范化路径)` 派生
/// （workbench-agent-wire-fix 2.4 / add-remote-execution-node 8.1）。
///
/// 项目身份是 `(节点, 路径)`——同一路径在两台机器上是两个项目，所以节点必须
/// 进材料，否则两台机器上的同名路径会被并成一个分组。
///
/// **归属**（`add-domain-layer` D5「放置规则」）：项目记录的唯一形态既是域对象
/// 又是 `projects` 表的行，因此它的身份规则与记录同处——定义在拥有该表的
/// crate，而不是中立层。落库的 id 与界面寻址的 id 必须是同一个值：两处各写一
/// 份哈希，漂移时不会有编译器或形状钉发现，症状恰好是「按 id 的写静默落空」。
/// 消费面（`sebas-webui::projects::project_id_for_on`）是薄委托。
///
/// `node_id` 为空白或等于 [`LOCAL_NODE_ID`] 时按本机处理（隐式本地注册的既有
/// 拼写：`(节点, 路径)` 的节点段省略）。
pub fn project_id_for_on(node_id: &str, canonical_path: &str) -> String {
    use sha2::{Digest, Sha256};
    let node_id = node_id.trim();
    let material = if node_id.is_empty() || node_id == LOCAL_NODE_ID {
        canonical_path.to_string()
    } else {
        // 节点标识不含 `:`（见 validate_node_id），因此第一个 `:` 就是分隔点。
        format!("{node_id}:{canonical_path}")
    };
    let digest = Sha256::digest(material.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("proj-{}", &hex[..12])
}

/// projects 表行 = 规范项目记录（存储 + 线同一形状）。列顺序即建表事实
/// （sqlite-auto-schema-sync）。
#[derive(
    Debug, Clone, PartialEq, Eq, SchemaColumns, ActiveRecord, serde::Serialize,
    serde::Deserialize,
)]
#[active_record(table = "projects")]
#[active_record(pk = "path")]
pub struct ProjectRow {
    /// 稳定项目 id（`proj-<12hex>`；workbench-agent-wire-fix 2.4）。派生自
    /// `(节点, 规范化路径)`（同文件的 [`project_id_for_on`]，与 webui 消费面
    /// 共用同一份实现），重启与重建后稳定；wire 不携带裸路径。
    ///
    /// **注册时即落库**（`add_project`）。`Option` 只为容忍迁移前的旧行
    /// （id 为 NULL）——读路径按 path 回填，重排落库后自愈。按 id 寻址的写
    /// （移除 / 重排 / 项目级默认 agent）依赖它有值：NULL 会让
    /// `UPDATE … WHERE id = ?` 匹配 0 行却返回 Ok（假装成功）。
    pub id: Option<String>,
    pub path: String,
    pub name: String,
    /// Unix seconds of the last branch probe (0 = never).
    #[serde(default)]
    #[column(default = "0")]
    pub branch_at: i64,
    pub added_at: i64,
    /// 注册表列表的展示顺序（`sort_order, added_at` 排序）。
    #[serde(default)]
    #[column(default = "0")]
    pub sort_order: i64,
    /// 项目所在的**执行节点**（add-remote-execution-node 3.2）。
    ///
    /// 项目身份是 `(节点, 路径)`：同一个路径在两台机器上是**两个项目**，因此 id
    /// 也必须随节点不同（webui 侧 `project_id_for_on` 派生）。缺省回填
    /// [`LOCAL_NODE_ID`]——那就是迁移，不需要额外的迁移脚本。
    #[serde(default = "default_node_id")]
    #[column(default = "'local'")]
    pub node_id: String,
    /// The agent id most recently used to create a session under this
    /// project (project-level default agent, workbench-agent-wire-fix D5).
    /// `None` = the operator has not created a session here yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
    /// Git branch read lazily; refreshed at most once per `BRANCH_TTL_SECS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

fn default_node_id() -> String {
    LOCAL_NODE_ID.to_string()
}

impl ProjectRow {
    /// 是否注册在本机节点上。
    ///
    /// 只有本机项目才能做本地文件系统操作（`is_accessible` / 分支探测 / 目录浏览）；
    /// 远端项目的路径可用性由**节点在 spawn 时**判定（3.3），主控不做本地 stat。
    pub fn is_local(&self) -> bool {
        self.node_id == LOCAL_NODE_ID
    }
}

/// 规范项目记录的线形状别名（migrate-project-registry 2.1：`ProjectEntry`
/// 与 `ProjectRow` 合一——`ProjectEntry` 曾是 `sebas-domain` 里的独立线形状，
/// 现由本定义再导出，调用点零改动）。
pub type ProjectEntry = ProjectRow;

#[cfg(test)]
mod tests {
    use super::*;

    /// 稳定 id 的形状与「节点进材料」的语义：本机拼写（节点省略）与显式
    /// `local` 同值，远端节点不同值——这是 `(节点, 路径)` 身份的核心断言。
    ///
    /// 该测试原在 `sebas-domain::project`；随放置规则（add-domain-layer D5）
    /// 与派生本身一起迁到记录所在处，顺带钉住唯一来源——域侧那份重复常量已删。
    #[test]
    fn stable_project_id_is_derived_from_node_and_path() {
        let local = project_id_for_on("local", "/data/work");
        assert_eq!(local, project_id_for_on("", "/data/work"));
        assert_eq!(local, project_id_for_on("  ", "/data/work"));
        assert_eq!(local, project_id_for_on("local", "/data/work"));
        assert_eq!(local.len(), "proj-".len() + 12);
        assert!(local.starts_with("proj-"));
        // 同路径不同节点 → 不同 id；同节点不同路径 → 不同 id。
        assert_ne!(local, project_id_for_on("dev-box", "/data/work"));
        assert_ne!(local, project_id_for_on("local", "/data/other"));
        // 确定性：同输入恒等（重启/重建后稳定）。
        assert_eq!(local, project_id_for_on("local", "/data/work"));
    }

    /// add_project 落库时写 id 的行为**不在这里测**：那需要一张 `projects`
    /// 表，而手写 `CREATE TABLE` 只允许出现在根 crate 的注册表（AGENTS.md 的
    /// 持久层准入规则 3）。覆盖在根 crate 的 `tests/state_persistence_test.rs`
    /// 与 e2e 旅程 `single_state_dir_journey_pins_every_state_location`
    /// （断言注册行的 id 落库即为 `proj-` 前缀）。

    fn row() -> ProjectRow {
        ProjectRow {
            id: Some("proj-abcdef123456".into()),
            path: "/data/work".into(),
            name: "work".into(),
            branch_at: 1_700_000_100,
            added_at: 1_700_000_000,
            sort_order: 3,
            node_id: LOCAL_NODE_ID.into(),
            default_agent: Some("claude".into()),
            branch: Some("main".into()),
        }
    }

    /// 形状钉（存储 + 线同一形状的两侧拼写）：可选字段缺席不序列化、
    /// node_id 缺省 `local`、旧最小条目反序列化回填——给规范定义加字段
    /// 即本测试失败（两侧一起变，无法只改一边）。
    #[test]
    fn project_record_serialized_shape_is_pinned() {
        // 完整条目（远端节点，可选字段在场）。
        let mut full = row();
        full.node_id = "node-b".into();
        assert_eq!(
            serde_json::to_value(&full).unwrap(),
            serde_json::json!({
                "id": "proj-abcdef123456",
                "path": "/data/work",
                "name": "work",
                "branch_at": 1_700_000_100,
                "added_at": 1_700_000_000,
                "sort_order": 3,
                "node_id": "node-b",
                "default_agent": "claude",
                "branch": "main",
            })
        );

        // 本机条目：可选字段缺席（skip_serializing_if），node_id 原样上 wire
        // （default 回填后的拼写保持 `local`，与合并前 ProjectEntry 一致）。
        let mut local = row();
        local.default_agent = None;
        local.branch = None;
        local.sort_order = 0;
        assert_eq!(
            serde_json::to_value(&local).unwrap(),
            serde_json::json!({
                "id": "proj-abcdef123456",
                "path": "/data/work",
                "name": "work",
                "branch_at": 1_700_000_100,
                "added_at": 1_700_000_000,
                "sort_order": 0,
                "node_id": "local",
            })
        );

        // 旧注册表条目（无 id / 无可选字段 / 无 node_id）：缺省回填，不报错。
        let legacy: ProjectRow = serde_json::from_str(
            r#"{"path": "/tmp/p2", "name": "p2", "added_at": 5}"#,
        )
        .unwrap();
        assert_eq!(legacy.id, None);
        assert_eq!(legacy.node_id, LOCAL_NODE_ID);
        assert!(legacy.is_local());
    }

    /// 列元数据形状钉（4.5 基线 + node_id 列）：列名/亲和/默认值/可空性。
    #[test]
    fn schema_columns_match_target_shape() {
        use sebas_db::record::Record;
        use sebas_db::schema::SchemaColumn;
        assert_eq!(ProjectRow::TABLE, "projects");
        assert_eq!(ProjectRow::PK_COLUMNS, &["path"]);
        let baseline: &[SchemaColumn] = &[
            SchemaColumn { name: "id", affinity: "TEXT", default: None, not_null: false, rename_from: None },
            SchemaColumn { name: "path", affinity: "TEXT", default: None, not_null: true, rename_from: None },
            SchemaColumn { name: "name", affinity: "TEXT", default: None, not_null: true, rename_from: None },
            SchemaColumn { name: "branch_at", affinity: "INTEGER", default: Some("0"), not_null: true, rename_from: None },
            SchemaColumn { name: "added_at", affinity: "INTEGER", default: None, not_null: true, rename_from: None },
            SchemaColumn { name: "sort_order", affinity: "INTEGER", default: Some("0"), not_null: true, rename_from: None },
            SchemaColumn { name: "node_id", affinity: "TEXT", default: Some("'local'"), not_null: true, rename_from: None },
            SchemaColumn { name: "default_agent", affinity: "TEXT", default: None, not_null: false, rename_from: None },
            SchemaColumn { name: "branch", affinity: "TEXT", default: None, not_null: false, rename_from: None },
        ];
        assert_eq!(ProjectRow::schema_columns(), baseline);
        assert_eq!(ProjectRow::COLUMNS, baseline.iter().map(|c| c.name).collect::<Vec<_>>().as_slice());
    }

    /// migrate-project-registry 2.5：展示层计算字段不进记录——注册表不携带
    /// 仅用于呈现的字段（分支缓存 `branch`/`branch_at` 是数据本身，不算）。
    #[test]
    fn record_carries_no_presentation_only_fields() {
        use sebas_db::record::Record;
        let fields: Vec<&str> = ProjectRow::COLUMNS.to_vec();
        let presentation = ["status_label", "accessible", "branch_display"];
        for f in fields {
            assert!(
                !presentation.contains(&f),
                "展示字段 {f} 不应进入规范记录"
            );
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
///
/// migrate-project-registry 收口：**id 在落库时就写入**（由同文件的
/// [`project_id_for_on`] 按 `(节点, 规范化路径)` 派生）。
/// 曾经这里落 `id: None`、只在 webui 读路径回填，后果是按 id 寻址的操作
/// （移除 / 重排 / 项目级默认 agent）对真实库行全部落空——`UPDATE … WHERE
/// id = ?` 匹配 0 行还返回 Ok，即「假装成功」。迁移前的旧行（id 为 NULL）仍
/// 由读路径回填容忍。
pub fn add_project(
    conn: &mut Connection,
    node_id: &str,
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
        id: Some(project_id_for_on(node_id, path)),
        path: path.to_string(),
        name: name.to_string(),
        branch_at: 0,
        added_at,
        sort_order: 0,
        node_id: node_id.to_string(),
        default_agent: None,
        branch: None,
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
