//! 启动 schema 同步：model struct 即事实源，结构差异一律**原位保数据迁移**
//! （retire-schema-reset；取代 sqlite-auto-schema-sync 的「不兼容即删库重建」）。
//!
//! # 同步流程
//!
//! 1. 先 [`conn::open`]。打不开 = 库损坏 → 走既有损坏拒启路径，**绝不删除
//!    文件、也绝不重建**（损坏与结构漂移严格分离）。
//! 2. **计划构建**（只读，design D1a）：全新库（无任何用户表）→ 逐表建
//!    schema；存量库 → 逐注册表 diff 派生列 vs `PRAGMA table_info`，解析
//!    `rename_from` 改名对、预判受限列（索引 / 主键 / 外键引用），产出带序
//!    动作清单（CreateTable / Add / Rename / Rebuild / Drop / 孤儿临时表清理）。
//!    `version_format` 缺失或未知**只 WARN**，照常 reconcile（旧迁移链库由
//!    此被吸纳；版本键纯诊断，结构对比是唯一动作触发）。
//! 3. **备份**（design D1b/D5）：计划含破坏性动作（重建 / 删列 / 孤儿表清理）
//!    时，先 `VACUUM INTO '<db>.pre-sync'`（覆盖式单文件，不能在事务内跑）；
//!    备份失败 → 拒启动、不动库。
//! 4. **单事务执行**（design D1c）：全部动作 + 版本 stamp 一提交全生效。
//!    - 缺列且能安全补（可空，或非空带常量默认）→ `ALTER TABLE ADD COLUMN`；
//!    - 声明改名（`rename_from` 命中 live 旧列）→ `ALTER TABLE RENAME COLUMN`，
//!      未命中的标注退化为普通补列 + WARN；未标注的「一缺一多」按删 + 加，
//!      **不猜**改名；
//!    - 类型不符 / 受限删列 → 事务内按注册 DDL 覆盖式重建（临时名建新表 →
//!      按列名交集拷贝存量行 → 换名 → 按注册索引段重建索引），数据随列名保全；
//!    - 多余列（未被索引/主键/外键引用）→ `ALTER TABLE DROP COLUMN`，数据随列
//!      丢弃，日志点名表与列；
//!    - 整个表缺失 → 按注册 DDL 建表。
//!    中途失败 → 事务回滚 + 拒启动 + 诊断点名失败步骤；数据库字节不变。
//! 5. 任何路径都**不删除、不重建数据库文件**；`SyncOutcome` 只报告结构化动作
//!    计数（design D6/D7），不再有重置出口。
//!
//! 注册表（`&[TableSchema]`）由调用方传入——哪些表、什么约束是**域 schema
//! 事实**，留在域侧（根 crate 注册表）；本模块只知道「怎么比、怎么建、怎么
//! 迁移」。

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::conn;

/// 当前 schema 版本 (sqlite-auto-schema-sync D4): 日期常量, 改 schema 时
/// bump, 不是 wall-clock。作用是诊断"这库是哪个 schema 日期的"——**纯诊断**,
/// 不触发任何同步动作 (retire-schema-reset D6)。
pub const SCHEMA_VERSION: &str = "20260924";

/// 版本键的格式标识。缺失/未知**不再重置**：WARN + 照常按结构 reconcile
/// （为发布后的迁移机制预留格式位）。
pub const VERSION_FORMAT: &str = "date";

/// 覆盖式重建的临时表名后缀（`<table>__sync_tmp`）。
const TEMP_TABLE_SUFFIX: &str = "__sync_tmp";

/// 破坏性迁移前的备份文件后缀（`<db>.pre-sync`，覆盖式单文件）。
pub const BACKUP_SUFFIX: &str = ".pre-sync";

/// 同步层自建自管的版本元数据表 (键值对)。不参与 diff。
/// DDL 用小写：这是 runtime 自己的家务表、不是域 schema——域 DDL 的
/// 「只在根注册表」机械门禁（extract-sebas-db 4.6 的大写建表关键词检查）
/// 因此保持可查且干净。
const SCHEMA_META_DDL: &str =
    "create table if not exists schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)";

/// 单列元数据 (由 `#[derive(SchemaColumns)]` 生成, 挂在每个 `*Row` struct 上)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaColumn {
    pub name: &'static str,
    /// SQLite 亲和类型: TEXT / INTEGER / REAL / BLOB / NUMERIC。
    pub affinity: &'static str,
    /// 常量默认值 (逐字作为 SQL `DEFAULT` 表达式), 仅缺列补齐时使用。
    pub default: Option<&'static str>,
    pub not_null: bool,
    /// 显式改名来源（旧列名）。`Some(old)` = 本列由 `old` 改名而来，启动同步
    /// 执行 `ALTER TABLE RENAME COLUMN old TO name` 保数据；`None` = 从未改名。
    /// **不猜**：未标注的「一缺一多」按删列 + 补列处理 (retire-schema-reset D2)。
    pub rename_from: Option<&'static str>,
}

/// 表注册三元组（retire-schema-reset D3 拆分）：表名 + 建表 DDL + 索引 DDL 段
/// + 派生列清单。
///
/// `create_table_ddl` 与各 `index_ddls` 都是**完整语句**，首建时逐字执行；覆盖式
/// 重建时建表段以临时表名重新组装（`CREATE TABLE <表名>` 头部替换，其余逐字
/// 保留），索引段在换名之后逐字执行——所以拆段对首建布局零影响。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSchema {
    pub name: &'static str,
    /// 建表语句（不含索引段）。
    pub create_table_ddl: &'static str,
    /// 该表的索引建立语句（首建 / 重建后执行）。
    pub index_ddls: &'static [&'static str],
    pub columns: &'static [SchemaColumn],
}

/// 同步结果 (观测/日志/测试用)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// 全新库: 按注册 DDL 建 schema 并打版本键。
    FreshCreated,
    /// 结构一致, 无需变更 (版本值若有差异仅随写 meta)。
    UpToDate,
    /// 有结构差异，已按原位保数据迁移执行完毕（计数即实际动作数）。
    Synced {
        added_columns: usize,
        renamed: usize,
        rebuilt: usize,
        dropped: usize,
    },
}

/// 打开数据库并同步 schema。这是写者线程的启动入口。
///
/// `open` 失败按损坏拒启且不动文件；成功打开后跑 reconcile——任何结构差异都
/// 原位迁移，任何失败都回滚 + 拒启动，**没有任何删库出口**。
pub fn open_and_sync(
    db_path: &Path,
    tables: &[TableSchema],
) -> Result<(Connection, SyncOutcome), String> {
    let conn = conn::open(db_path).map_err(|e| {
        format!(
            "打开状态库失败，疑似损坏，拒绝启动且不自动删除文件: {} ({e})",
            db_path.display()
        )
    })?;

    sync_conn(conn, db_path, tables)
}

/// 对已打开的连接执行同步（计划 → 备份 → 单事务执行）。
pub fn sync_conn(
    conn: Connection,
    db_path: &Path,
    tables: &[TableSchema],
) -> Result<(Connection, SyncOutcome), String> {
    let mut conn = conn;

    // 全新库: 没有任何用户表 → 直接按注册 DDL 建 schema（不必删文件）。
    let fresh = list_user_tables(&conn)
        .map_err(|e| format!("读取 sqlite_master 失败: {e}"))?
        .is_empty();

    // 版本键纯诊断: 缺失/未知只 WARN, 照常 reconcile（覆盖旧迁移链库——它们
    // 只有 user_version、无 schema_meta; 查询遇"无此表"同样返回 None）。
    if !fresh {
        let format: Option<String> = conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'version_format'",
                [],
                |row| row.get(0),
            )
            .ok();
        if format.as_deref() != Some(VERSION_FORMAT) {
            warn!(
                path = %db_path.display(),
                version_format = ?format,
                expected = VERSION_FORMAT,
                "版本元数据缺失或未知格式: 照常按结构 reconcile（不重置、不删库）"
            );
        }
    }

    let plan = build_plan(&conn, tables, fresh)?;

    if plan.destructive {
        backup_database(&conn, db_path).map_err(|e| {
            format!(
                "破坏性 schema 迁移前的备份失败，拒绝执行迁移且不启动: {e} \
                 (库 {})",
                db_path.display()
            )
        })?;
    }

    if fresh {
        rebuild_schema(&mut conn, tables)?;
        return Ok((conn, SyncOutcome::FreshCreated));
    }

    if plan.actions.is_empty() {
        // 结构一致: 版本值差异不触发任何动作, 仅随写 meta 更新。
        run_in_transaction(&mut conn, &plan, false)?;
        return Ok((conn, SyncOutcome::UpToDate));
    }

    execute_plan(&mut conn, &plan)?;
    Ok((
        conn,
        SyncOutcome::Synced {
            added_columns: plan.added_columns,
            renamed: plan.renamed,
            rebuilt: plan.rebuilt,
            dropped: plan.dropped,
        },
    ))
}

// ---- 计划构建（design D1a：只读）----

/// 一条待执行的结构迁移动作（有序）。
enum PlanAction<'a> {
    /// 整表缺失 → 按注册 DDL 建表。
    CreateTable(&'a TableSchema),
    /// 上一轮未完成重建遗留的临时表（同事务里先清掉）。
    DropOrphanTemp { table: &'a str, temp: String },
    Add {
        table: &'a str,
        col: &'a SchemaColumn,
    },
    Rename {
        table: &'a str,
        old: String,
        new: &'a str,
    },
    /// 类型不符 / 受限删列 → 覆盖式重建（数据按列名交集保全）。
    Rebuild { table: &'a TableSchema, reason: String },
    Drop {
        table: &'a str,
        name: String,
    },
}

impl PlanAction<'_> {
    /// 诊断用的步骤名（失败时点名失败步骤）。
    fn label(&self) -> String {
        match self {
            PlanAction::CreateTable(t) => format!("建表 {}", t.name),
            PlanAction::DropOrphanTemp { table, temp } => {
                format!("清理孤儿临时表 {table} -> {temp}")
            }
            PlanAction::Add { table, col } => format!("补列 {table}.{}", col.name),
            PlanAction::Rename { table, old, new } => {
                format!("改名 {table}.{old} -> {new}")
            }
            PlanAction::Rebuild { table, reason } => {
                format!("重建表 {}（{reason}）", table.name)
            }
            PlanAction::Drop { table, name } => format!("删列 {table}.{name}"),
        }
    }
}

#[derive(Default)]
struct Plan<'a> {
    /// 含破坏性动作（重建 / 删列 / 孤儿表清理）→ 执行前必须备份。
    destructive: bool,
    actions: Vec<PlanAction<'a>>,
    added_columns: usize,
    renamed: usize,
    rebuilt: usize,
    dropped: usize,
}

/// 只读构建迁移计划：不做任何写入。
fn build_plan<'a>(
    conn: &Connection,
    tables: &'a [TableSchema],
    fresh: bool,
) -> Result<Plan<'a>, String> {
    let mut plan = Plan::default();
    if fresh {
        for table in tables {
            plan.actions.push(PlanAction::CreateTable(table));
        }
        return Ok(plan);
    }
    for table in tables {
        plan_table(conn, table, &mut plan)?;
    }
    Ok(plan)
}

/// 单表 diff → 动作（rename 解析、受限预判、重建/快路径分派）。
fn plan_table<'a>(
    conn: &Connection,
    table: &'a TableSchema,
    plan: &mut Plan<'a>,
) -> Result<(), String> {
    let live = live_columns(conn, table.name)
        .map_err(|e| format!("读取 {} 表结构失败: {e}", table.name))?;

    if live.is_empty() {
        info!(table = table.name, "注册表缺失: 按注册 DDL 建表（不触碰其它表）");
        plan.actions.push(PlanAction::CreateTable(table));
        return Ok(());
    }

    // 上一轮未完成重建的临时表（单事务不可能提交过，但作为兜底仍先清掉）。
    let temp = temp_table_name(table.name);
    if user_table_exists(conn, &temp)
        .map_err(|e| format!("检查临时表 {temp} 失败: {e}"))?
    {
        warn!(
            table = table.name,
            temp = %temp,
            "发现上一轮遗留的重建临时表: 同事务内先清理"
        );
        plan.actions.push(PlanAction::DropOrphanTemp {
            table: table.name,
            temp,
        });
        plan.destructive = true;
    }

    // ---- 改名对解析（显式 rename_from，命中 live 旧列且新列尚不存在）----
    let mut renames: Vec<(String, &'a SchemaColumn)> = Vec::new();
    for col in table.columns {
        let Some(old) = col.rename_from else {
            continue;
        };
        let has_old = live.iter().any(|l| l.name == old);
        let has_new = live.iter().any(|l| l.name == col.name);
        if has_old && !has_new {
            renames.push((old.to_string(), col));
        } else if !has_old && !has_new {
            warn!(
                table = table.name,
                column = col.name,
                rename_from = old,
                "声明的改名来源列不存在（新装库/已迁过）: 退化为普通补列"
            );
        } else if has_old {
            warn!(
                table = table.name,
                column = col.name,
                rename_from = old,
                "库内新旧列同时存在: 不做改名，旧列交给多余列处理"
            );
        }
    }

    // ---- 缺列 / 类型不符 / 多余列 ----
    let mut missing: Vec<&'a SchemaColumn> = Vec::new();
    for col in table.columns {
        if !live
            .iter()
            .any(|l| effective_name(l, &renames) == col.name)
        {
            missing.push(col);
        }
    }
    // 补不了的缺列（非空且无常量默认）不硬来，也不删库：fail-closed 拒启动。
    for col in &missing {
        if col.not_null && col.default.is_none() {
            return Err(format!(
                "表 {} 缺列 `{}`: 非空且无常量默认值，无法原地补列、也无法重建回填；\
                 拒绝启动并保持数据库原样（删库重置已废除）",
                table.name, col.name
            ));
        }
    }

    let mut type_mismatch: Option<String> = None;
    for col in table.columns {
        if let Some(l) = live
            .iter()
            .find(|l| effective_name(l, &renames) == col.name)
        {
            let (live_affinity, want_affinity) =
                (type_affinity(&l.decl), type_affinity(col.affinity));
            if live_affinity != want_affinity {
                type_mismatch = Some(format!(
                    "列 `{}` 类型不符: 库内 `{}` (亲和 {live_affinity}), model 期望亲和 {want_affinity}",
                    col.name, l.decl
                ));
                break;
            }
        }
    }

    let extra: Vec<&LiveColumn> = live
        .iter()
        .filter(|l| {
            !table
                .columns
                .iter()
                .any(|c| c.name == effective_name(l, &renames))
        })
        .collect();

    // 受限预判：被索引 / 主键 / 外键引用的多余列不能直接 DROP COLUMN。
    let mut restricted: Vec<&LiveColumn> = Vec::new();
    for l in &extra {
        if column_is_referenced(conn, table.name, &l.name)? {
            restricted.push(l);
        }
    }

    if let Some(reason) = type_mismatch {
        warn!(table = table.name, reason = %reason, "schema 漂移: 事务内重建表，按列名交集保数据（先备份）");
        return Ok(push_rebuild(plan, table, renames, extra, missing, reason));
    }
    if !restricted.is_empty() {
        let names = restricted
            .iter()
            .map(|l| format!("`{}`", l.name))
            .collect::<Vec<_>>()
            .join(", ");
        let reason = format!("多余列 {names} 被索引/主键/外键引用，不能直接 DROP COLUMN");
        warn!(table = table.name, reason = %reason, "schema 漂移: 事务内重建表（先备份）");
        return Ok(push_rebuild(plan, table, renames, extra, missing, reason));
    }

    // ---- 非破坏性动作：先改名，再补列，最后删列 ----
    for (old, col) in renames {
        warn!(table = table.name, old = %old, new = col.name, "声明改名: RENAME COLUMN 保数据");
        plan.actions.push(PlanAction::Rename {
            table: table.name,
            old,
            new: col.name,
        });
        plan.renamed += 1;
    }
    for col in missing {
        info!(table = table.name, column = col.name, default = ?col.default, "缺列: ALTER TABLE ADD COLUMN 原地补齐");
        plan.actions.push(PlanAction::Add {
            table: table.name,
            col,
        });
        plan.added_columns += 1;
    }
    for l in extra {
        warn!(
            table = table.name,
            column = %l.name,
            "多余列: DROP COLUMN（数据随列丢弃，先备份）"
        );
        plan.actions.push(PlanAction::Drop {
            table: table.name,
            name: l.name.clone(),
        });
        plan.dropped += 1;
        plan.destructive = true;
    }
    Ok(())
}

/// 重建动作入计划：先执行改名（重建的列名交集据此看到新列名），再重建。
/// 重建的拷贝按「model 列 ∩ live 列」交集进行，故多余列随重建丢弃、缺列落
/// 注册 DDL 的默认值——与快路径语义一致，计数逐项登记。
fn push_rebuild<'a>(
    plan: &mut Plan<'a>,
    table: &'a TableSchema,
    renames: Vec<(String, &'a SchemaColumn)>,
    extra: Vec<&LiveColumn>,
    missing: Vec<&'a SchemaColumn>,
    reason: String,
) {
    for (old, col) in renames {
        warn!(table = table.name, old = %old, new = col.name, "声明改名: RENAME COLUMN 保数据（紧随重建）");
        plan.actions.push(PlanAction::Rename {
            table: table.name,
            old,
            new: col.name,
        });
        plan.renamed += 1;
    }
    for l in &extra {
        warn!(
            table = table.name,
            column = %l.name,
            "多余列随重建丢弃（数据随列丢弃）"
        );
        plan.dropped += 1;
    }
    for col in &missing {
        info!(
            table = table.name,
            column = col.name,
            default = ?col.default,
            "缺列随重建按注册 DDL 默认值补齐"
        );
        plan.added_columns += 1;
    }
    plan.actions.push(PlanAction::Rebuild { table, reason });
    plan.rebuilt += 1;
    plan.destructive = true;
}

/// live 列在「改名已生效」后的有效列名。
fn effective_name<'l, 'c: 'l>(
    live: &'l LiveColumn,
    renames: &'l [(String, &'c SchemaColumn)],
) -> &'l str {
    renames
        .iter()
        .find(|(old, _)| *old == live.name)
        .map(|(_, col)| col.name)
        .unwrap_or(&live.name)
}

/// 多余列是否被索引 / 主键 / 外键引用（受限删列 → 升级为表重建）。
fn column_is_referenced(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    // 主键成员（含 INTEGER PRIMARY KEY 的 rowid 别名形态）。
    let pk: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(pk), 0) FROM pragma_table_info(?1) WHERE name = ?2",
            rusqlite::params![table, column],
            |r| r.get(0),
        )
        .map_err(|e| format!("读取 {table} 主键信息失败: {e}"))?;
    if pk > 0 {
        return Ok(true);
    }

    // 索引成员（含 UNIQUE / PRIMARY KEY 自动索引）。
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_index_list(?1)")
        .map_err(|e| format!("读取 {table} 索引清单失败: {e}"))?;
    let index_names: Vec<Option<String>> = stmt
        .query_map([table], |r| r.get(0))
        .map_err(|e| format!("读取 {table} 索引清单失败: {e}"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| format!("读取 {table} 索引清单失败: {e}"))?;
    for index in index_names.into_iter().flatten() {
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_index_info(?1)")
            .map_err(|e| format!("读取索引 {index} 列清单失败: {e}"))?;
        let cols: Vec<String> = stmt
            .query_map([&index], |r| r.get(0))
            .map_err(|e| format!("读取索引 {index} 列清单失败: {e}"))?
            .collect::<rusqlite::Result<_>>()
            .map_err(|e| format!("读取索引 {index} 列清单失败: {e}"))?;
        if cols.iter().any(|c| c == column) {
            return Ok(true);
        }
    }

    // 外键 from 列。
    let mut stmt = conn
        .prepare("SELECT \"from\" FROM pragma_foreign_key_list(?1)")
        .map_err(|e| format!("读取 {table} 外键清单失败: {e}"))?;
    let fk_cols: Vec<String> = stmt
        .query_map([table], |r| r.get(0))
        .map_err(|e| format!("读取 {table} 外键清单失败: {e}"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| format!("读取 {table} 外键清单失败: {e}"))?;
    Ok(fk_cols.iter().any(|c| c == column))
}

// ---- 备份（design D1b/D5）----

/// 破坏性迁移前的整库备份：`VACUUM INTO '<db>.pre-sync'`（3.27+ 一致性快照）。
/// 目标已存在则先删后建（覆盖式单文件，不堆积）。失败原样上报 → 调用方拒启动。
fn backup_database(conn: &Connection, db_path: &Path) -> Result<PathBuf, String> {
    let target = db_sidecar_path(db_path, BACKUP_SUFFIX);
    if target.exists() {
        std::fs::remove_file(&target).map_err(|e| {
            format!("无法覆盖既有备份 {}: {e}", target.display())
        })?;
    }
    let target_str = target.to_string_lossy().to_string();
    conn.execute("VACUUM INTO ?1", rusqlite::params![target_str])
        .map_err(|e| format!("写入备份 {} 失败: {e}", target.display()))?;
    info!(path = %db_path.display(), backup = %target.display(), "破坏性 schema 迁移前已备份整库");
    Ok(target)
}

// ---- 事务执行（design D1c：单事务，全程原子）----

fn execute_plan(conn: &mut Connection, plan: &Plan) -> Result<(), String> {
    // `PRAGMA foreign_keys` 在事务内是 no-op，必须在开事务前切（design D3）。
    let fk_off = plan
        .actions
        .iter()
        .any(|a| matches!(a, PlanAction::Rebuild { .. }));
    if fk_off {
        conn.execute_batch("PRAGMA foreign_keys = OFF")
            .map_err(|e| format!("关闭外键约束失败（重建前置）: {e}"))?;
    }
    let result = run_in_transaction(conn, plan, fk_off);
    if fk_off {
        if let Err(e) = conn.execute_batch("PRAGMA foreign_keys = ON") {
            warn!(error = %e, "恢复外键约束失败（连接即将关闭，不影响已提交结果）");
        }
    }
    result
}

fn run_in_transaction(
    conn: &mut Connection,
    plan: &Plan,
    check_foreign_keys: bool,
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("schema 迁移事务开始失败: {e}"))?;

    for (idx, action) in plan.actions.iter().enumerate() {
        apply_action(&tx, action)
            .map_err(|e| format!("schema 迁移第 {} 步失败（{}）: {e}", idx + 1, action.label()))?;
    }

    // 重建后校验外键完整性（design D3 第 10 步）：违规即回滚拒启动，绝不带病提交。
    if check_foreign_keys {
        let violations: i64 = tx
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .map_err(|e| format!("重建后外键校验执行失败: {e}"))?;
        if violations > 0 {
            return Err(format!(
                "重建后外键校验发现 {violations} 条违规行: 回滚并拒绝启动"
            ));
        }
    }

    ensure_schema_meta(&tx)?;
    stamp_version(&tx)?;
    tx.commit()
        .map_err(|e| format!("schema 迁移事务提交失败: {e}"))?;
    Ok(())
}

fn apply_action(conn: &Connection, action: &PlanAction) -> Result<(), String> {
    match action {
        PlanAction::CreateTable(table) => {
            conn.execute_batch(table.create_table_ddl)
                .map_err(|e| format!("建表 {} 失败: {e}", table.name))?;
            for index_ddl in table.index_ddls {
                conn.execute_batch(index_ddl)
                    .map_err(|e| format!("建索引失败: {e}"))?;
            }
            Ok(())
        }
        PlanAction::DropOrphanTemp { temp, .. } => conn
            .execute_batch(&format!("DROP TABLE {}", quote_ident(temp)))
            .map_err(|e| format!("清理孤儿临时表 {temp} 失败: {e}")),
        PlanAction::Add { table, col } => conn
            .execute_batch(&add_column_sql(table, col))
            .map_err(|e| format!("补列 {}.{} 失败: {e}", table, col.name)),
        PlanAction::Rename { table, old, new } => conn
            .execute_batch(&format!(
                "ALTER TABLE {} RENAME COLUMN {} TO {}",
                quote_ident(table),
                quote_ident(old),
                quote_ident(new)
            ))
            .map_err(|e| format!("改名 {table}.{old} -> {new} 失败: {e}")),
        PlanAction::Rebuild { table, .. } => rebuild_table(conn, table),
        PlanAction::Drop { table, name } => conn
            .execute_batch(&format!(
                "ALTER TABLE {} DROP COLUMN {}",
                quote_ident(table),
                quote_ident(name)
            ))
            .map_err(|e| format!("删列 {table}.{name} 失败: {e}")),
    }
}

/// 覆盖式重建单表（design D3 的 12 步配方，事务内）：
/// 以临时名建新表 → 按列名交集拷贝存量行 → DROP 原表 → 临时表换回原名 →
/// 按注册索引段重建索引。整个 sync 在一个事务里，失败即回滚。
fn rebuild_table(conn: &Connection, table: &TableSchema) -> Result<(), String> {
    let temp = temp_table_name(table.name);
    let temp_ddl = create_table_ddl_with_name(table.create_table_ddl, table.name, &temp)?;
    if user_table_exists(conn, &temp).map_err(|e| format!("检查临时表 {temp} 失败: {e}"))? {
        conn.execute_batch(&format!("DROP TABLE {}", quote_ident(&temp)))
            .map_err(|e| format!("清理残留临时表 {temp} 失败: {e}"))?;
    }
    conn.execute_batch(&temp_ddl)
        .map_err(|e| format!("以临时名建新表 {temp} 失败: {e}"))?;

    // 按列名交集拷贝存量行（SQLite 亲和自动转换）；新列落注册 DDL 的默认值。
    let live = live_columns(conn, table.name)
        .map_err(|e| format!("读取 {} 表结构失败: {e}", table.name))?;
    let common: Vec<&str> = table
        .columns
        .iter()
        .map(|c| c.name)
        .filter(|name| live.iter().any(|l| l.name == *name))
        .collect();
    if common.is_empty() {
        return Err(format!(
            "重建表 {} 失败: 旧表与 model 列名无交集，无法按列名拷贝存量行（拒绝静默丢行）",
            table.name
        ));
    }
    let cols = common
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute_batch(&format!(
        "INSERT INTO {} ({cols}) SELECT {cols} FROM {}",
        quote_ident(&temp),
        quote_ident(table.name)
    ))
    .map_err(|e| format!("重建表 {} 拷贝存量行失败: {e}", table.name))?;

    conn.execute_batch(&format!("DROP TABLE {}", quote_ident(table.name)))
        .map_err(|e| format!("重建表 {} 删除旧表失败: {e}", table.name))?;
    conn.execute_batch(&format!(
        "ALTER TABLE {} RENAME TO {}",
        quote_ident(&temp),
        quote_ident(table.name)
    ))
    .map_err(|e| format!("重建表 {} 换回原名失败: {e}", table.name))?;
    for index_ddl in table.index_ddls {
        conn.execute_batch(index_ddl)
            .map_err(|e| format!("重建表 {} 后建索引失败: {e}", table.name))?;
    }
    Ok(())
}

/// 以 `new_name` 组装同构建表语句：只在 `CREATE TABLE [IF NOT EXISTS] <表名>`
/// 头部替换表名标记，其余部分（含注释/列定义里的同名子串）逐字保留。
/// 头部与注册表名不符 → fail-closed（不做整段字符串替换——design D3）。
fn create_table_ddl_with_name(
    create_table_ddl: &str,
    table: &str,
    new_name: &str,
) -> Result<String, String> {
    let lower = create_table_ddl.to_ascii_lowercase();
    let keyword = "create table";
    let start = lower
        .find(keyword)
        .ok_or_else(|| format!("注册建表 DDL 缺少 `CREATE TABLE` 关键字: {create_table_ddl}"))?;
    if !create_table_ddl[..start].trim().is_empty() {
        return Err(format!(
            "注册建表 DDL 的 `CREATE TABLE` 必须出现在语句开头: {create_table_ddl}"
        ));
    }
    let mut cursor = start + keyword.len();
    let mut rest_lower = &lower[cursor..];
    if let Some(offset) = rest_lower.find("if not exists") {
        // 「if not exists」与 CREATE TABLE 之间只允许空白。
        if lower[cursor..cursor + offset].trim().is_empty() {
            cursor += offset + "if not exists".len();
            rest_lower = &lower[cursor..];
        }
    }
    let _ = rest_lower;
    let bytes = create_table_ddl.as_bytes();
    while cursor < bytes.len() && (bytes[cursor] as char).is_ascii_whitespace() {
        cursor += 1;
    }
    let rest = &create_table_ddl[cursor..];
    let name_len = rest
        .find(|c: char| c.is_ascii_whitespace() || c == '(')
        .ok_or_else(|| format!("注册建表 DDL 的表名后缺少空白或 `(`: {create_table_ddl}"))?;
    let found = &rest[..name_len];
    if found != table {
        return Err(format!(
            "注册建表 DDL 的建表名 `{found}` 与注册表名 `{table}` 不符（拒绝按猜测重组 DDL）"
        ));
    }
    let mut out = String::with_capacity(create_table_ddl.len() + new_name.len());
    out.push_str(&create_table_ddl[..cursor]);
    out.push_str(new_name);
    out.push_str(&create_table_ddl[cursor + name_len..]);
    Ok(out)
}

fn temp_table_name(table: &str) -> String {
    format!("{table}{TEMP_TABLE_SUFFIX}")
}

/// 按注册 DDL 建全部表 + 索引, 建 schema_meta 并打版本键 (单事务)。
fn rebuild_schema(conn: &mut Connection, tables: &[TableSchema]) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("建 schema 事务开始失败: {e}"))?;
    for table in tables {
        tx.execute_batch(table.create_table_ddl)
            .map_err(|e| format!("建表 {} 失败: {e}", table.name))?;
        for index_ddl in table.index_ddls {
            tx.execute_batch(index_ddl)
                .map_err(|e| format!("为表 {} 建索引失败: {e}", table.name))?;
        }
    }
    ensure_schema_meta(&tx)?;
    stamp_version(&tx)?;
    tx.commit()
        .map_err(|e| format!("建 schema 事务提交失败: {e}"))?;
    Ok(())
}

fn ensure_schema_meta(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(SCHEMA_META_DDL)
        .map_err(|e| format!("创建 schema_meta 失败: {e}"))
}

/// 写/更新版本键 (upsert)。结构一致时版本值差异仅经此更新, 不触发任何动作。
fn stamp_version(conn: &Connection) -> Result<(), String> {
    for (key, value) in [
        ("version_format", VERSION_FORMAT),
        ("version", SCHEMA_VERSION),
    ] {
        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )
        .map_err(|e| format!("写版本键 {key} 失败: {e}"))?;
    }
    Ok(())
}

/// 拼缺列补齐语句: `ALTER TABLE t ADD COLUMN col AFFINITY [NOT NULL] [DEFAULT d]`。
/// 列名/默认值都来自编译期派生的常量, 非运行时输入。
pub fn add_column_sql(table: &str, col: &SchemaColumn) -> String {
    let mut sql = format!(
        "ALTER TABLE {} ADD COLUMN {} {}",
        quote_ident(table),
        quote_ident(col.name),
        col.affinity
    );
    if col.not_null {
        sql.push_str(" NOT NULL");
    }
    if let Some(d) = col.default {
        sql.push_str(" DEFAULT ");
        sql.push_str(d);
    }
    sql
}

/// 双引号包裹标识符（内部双引号翻倍）。表名/列名都来自注册表或库内元数据，
/// 仍逐处引用以免异常命名破坏语句。
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

struct LiveColumn {
    name: String,
    /// 库内声明的列类型 (可能为空串, 如无类型列)。
    decl: String,
}

/// `PRAGMA table_info` 的 TVF 形式。表不存在时返回空 (调用方以空判定缺表)。
fn live_columns(conn: &Connection, table: &str) -> rusqlite::Result<Vec<LiveColumn>> {
    let mut stmt =
        conn.prepare("SELECT name, COALESCE(\"type\", '') FROM pragma_table_info(?1)")?;
    let rows = stmt.query_map([table], |row| {
        Ok(LiveColumn {
            name: row.get(0)?,
            decl: row.get(1)?,
        })
    })?;
    rows.collect()
}

/// SQLite 类型亲和规则 (<https://www.sqlite.org/datatype3.html> §3.1)。
/// 对比两侧都归一到亲和, `VARCHAR(255)` 与 `TEXT` 不误报。
pub fn type_affinity(decl: &str) -> &'static str {
    let d = decl.to_ascii_uppercase();
    if d.contains("INT") {
        "INTEGER"
    } else if d.contains("CHAR") || d.contains("CLOB") || d.contains("TEXT") {
        "TEXT"
    } else if d.contains("BLOB") || d.is_empty() {
        "BLOB"
    } else if d.contains("REAL") || d.contains("FLOA") || d.contains("DOUB") {
        "REAL"
    } else {
        "NUMERIC"
    }
}

fn list_user_tables(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect()
}

fn user_table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
}

fn db_sidecar_path(db_path: &Path, suffix: &str) -> PathBuf {
    let mut s = db_path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::TEST_TABLES;
    use tempfile::tempdir;

    // ---- 测试用注册表构造（中性表名 alpha/beta，无域词）----

    macro_rules! col {
        ($name:expr, $aff:expr, $default:expr, $not_null:expr) => {
            SchemaColumn {
                name: $name,
                affinity: $aff,
                default: $default,
                not_null: $not_null,
                rename_from: None,
            }
        };
        ($name:expr, $aff:expr, $default:expr, $not_null:expr, rename_from = $old:expr) => {
            SchemaColumn {
                name: $name,
                affinity: $aff,
                default: $default,
                not_null: $not_null,
                rename_from: Some($old),
            }
        };
    }

    const ALPHA_DDL: &str = "create table alpha (
        id    TEXT PRIMARY KEY,
        name  TEXT NOT NULL,
        note  TEXT,
        score INTEGER NOT NULL DEFAULT 0
    );";

    static ALPHA_COLUMNS: &[SchemaColumn] = &[
        col!("id", "TEXT", None, true),
        col!("name", "TEXT", None, true),
        col!("note", "TEXT", None, false),
        col!("score", "INTEGER", Some("0"), true),
    ];

    /// alpha + note 索引（重建后索引应被重建）。
    static ALPHA_INDEXED_TABLES: &[TableSchema] = &[TableSchema {
        name: "alpha",
        create_table_ddl: ALPHA_DDL,
        index_ddls: &["create index idx_alpha_note on alpha(note)"],
        columns: ALPHA_COLUMNS,
    }];

    /// 声明改名的 alpha：`name` 由旧列 `title` 改名而来。
    static RENAME_ALPHA_TABLES: &[TableSchema] = &[TableSchema {
        name: "alpha",
        create_table_ddl: ALPHA_DDL,
        index_ddls: &[],
        columns: &[
            col!("id", "TEXT", None, true),
            col!("name", "TEXT", None, true, rename_from = "title"),
            col!("note", "TEXT", None, false),
            col!("score", "INTEGER", Some("0"), true),
        ],
    }];

    /// model 删掉 `note` 的 alpha（删列场景）。
    static DROP_NOTE_TABLES: &[TableSchema] = &[TableSchema {
        name: "alpha",
        create_table_ddl: "create table alpha (
        id    TEXT PRIMARY KEY,
        name  TEXT NOT NULL,
        score INTEGER NOT NULL DEFAULT 0
    );",
        index_ddls: &[],
        columns: &[
            col!("id", "TEXT", None, true),
            col!("name", "TEXT", None, true),
            col!("score", "INTEGER", Some("0"), true),
        ],
    }];

    /// model 多出可空列 `tag` 的 alpha（未声明的一缺一多场景）。
    static PAIR_ALPHA_TABLES: &[TableSchema] = &[TableSchema {
        name: "alpha",
        create_table_ddl: "create table alpha (
        id    TEXT PRIMARY KEY,
        name  TEXT NOT NULL,
        note  TEXT,
        score INTEGER NOT NULL DEFAULT 0,
        tag   TEXT
    );",
        index_ddls: &[],
        columns: &[
            col!("id", "TEXT", None, true),
            col!("name", "TEXT", None, true),
            col!("note", "TEXT", None, false),
            col!("score", "INTEGER", Some("0"), true),
            col!("tag", "TEXT", None, false),
        ],
    }];

    /// model 多出非空无默认列 `req` 的 alpha（不可迁移 → fail-closed）。
    static UNADDABLE_ALPHA_TABLES: &[TableSchema] = &[TableSchema {
        name: "alpha",
        create_table_ddl: "create table alpha (
        id    TEXT PRIMARY KEY,
        name  TEXT NOT NULL,
        note  TEXT,
        score INTEGER NOT NULL DEFAULT 0,
        req   TEXT NOT NULL
    );",
        index_ddls: &[],
        columns: &[
            col!("id", "TEXT", None, true),
            col!("name", "TEXT", None, true),
            col!("note", "TEXT", None, false),
            col!("score", "INTEGER", Some("0"), true),
            col!("req", "TEXT", None, true),
        ],
    }];

    /// 带约束的 alpha：注册 DDL 的 `score >= 0` CHECK 用来注入数据拷贝失败
    /// （存量行 score = -5），驱动「失败回滚 + 拒启动」场景。
    static CHECKED_ALPHA_TABLES: &[TableSchema] = &[TableSchema {
        name: "alpha",
        create_table_ddl: "create table alpha (
        id    TEXT PRIMARY KEY,
        name  TEXT NOT NULL,
        note  TEXT,
        score INTEGER NOT NULL DEFAULT 0 CHECK (score >= 0)
    );",
        index_ddls: &[],
        columns: ALPHA_COLUMNS,
    }];

    /// 计划序列专用：`name` 由可空的 `title` 改名而来，多出可空列 `tag`。
    static PLAN_ALPHA_TABLES: &[TableSchema] = &[TableSchema {
        name: "alpha",
        create_table_ddl: "create table alpha (
        id    TEXT PRIMARY KEY,
        name  TEXT,
        note  TEXT,
        score INTEGER NOT NULL DEFAULT 0,
        tag   TEXT
    );",
        index_ddls: &[],
        columns: &[
            col!("id", "TEXT", None, true),
            col!("name", "TEXT", None, false, rename_from = "title"),
            col!("note", "TEXT", None, false),
            col!("score", "INTEGER", Some("0"), true),
            col!("tag", "TEXT", None, false),
        ],
    }];

    /// 改名来源缺失专用：可空列 `note` 声明由 `old_note` 改名而来。
    static RENAME_MISS_TABLES: &[TableSchema] = &[TableSchema {
        name: "alpha",
        create_table_ddl: "create table alpha (
        id    TEXT PRIMARY KEY,
        name  TEXT NOT NULL,
        note  TEXT,
        score INTEGER NOT NULL DEFAULT 0
    );",
        index_ddls: &[],
        columns: &[
            col!("id", "TEXT", None, true),
            col!("name", "TEXT", None, true),
            col!("note", "TEXT", None, false, rename_from = "old_note"),
            col!("score", "INTEGER", Some("0"), true),
        ],
    }];

    /// 两表注册：alpha（类型漂移 → 重建）+ beta（形状一致，必须不被触碰）。
    static TWO_TABLE_REGISTRY: &[TableSchema] = &[
        TableSchema {
            name: "alpha",
            create_table_ddl: ALPHA_DDL,
            index_ddls: &[],
            columns: ALPHA_COLUMNS,
        },
        TableSchema {
            name: "beta",
            create_table_ddl: "create table beta (k TEXT PRIMARY KEY, v TEXT NOT NULL);",
            index_ddls: &[],
            columns: &[
                col!("k", "TEXT", None, true),
                col!("v", "TEXT", None, true),
            ],
        },
    ];

    fn temp_db(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let path = dir.path().join(name);
        (dir, path)
    }

    fn meta_get(conn: &Connection, key: &str) -> Option<String> {
        conn.query_row(
            "SELECT value FROM schema_meta WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .ok()
    }

    /// 测试用: 直接写 schema_meta 键 (模拟旧值/坏格式)。
    fn meta_set(conn: &Connection, key: &str, value: &str) {
        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            [key, value],
        )
        .unwrap();
    }

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect()
    }

    fn column_names(conn: &Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect()
    }

    fn column_decl(conn: &Connection, table: &str, column: &str) -> String {
        conn.query_row(
            &format!("SELECT COALESCE(\"type\", '') FROM pragma_table_info('{table}') WHERE name = '{column}'"),
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn index_names(conn: &Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("SELECT name FROM pragma_index_list('{table}')"))
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect()
    }

    fn existing_files_with(dir: &Path, needle: &str) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_string_lossy().contains(needle))
            .collect();
        files.sort();
        files
    }

    fn backup_files(dir: &Path) -> Vec<PathBuf> {
        existing_files_with(dir, BACKUP_SUFFIX)
    }

    fn reset_files(dir: &Path) -> Vec<PathBuf> {
        existing_files_with(dir, ".reset-")
    }

    /// 建 `alpha` 的旧结构库（4 列现行形状）+ schema_meta 版本键 + 一行业务行。
    fn seed_alpha(path: &Path, insert_sql: &str) {
        let conn = conn::open(path).unwrap();
        conn.execute_batch(&format!("{ALPHA_DDL}\n{SCHEMA_META_DDL}"))
            .unwrap();
        stamp_version(&conn).unwrap();
        conn.execute_batch(insert_sql).unwrap();
    }

    /// 文件快照（字节 + mtime）——失败迁移必须与之逐项相等。
    fn snapshot(path: &Path) -> (Vec<u8>, std::time::SystemTime) {
        (
            std::fs::read(path).unwrap(),
            std::fs::metadata(path).unwrap().modified().unwrap(),
        )
    }

    /// 测试日志捕获的共享缓冲 (fmt subscriber 的 writer)。
    #[derive(Clone)]
    struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    // ---- spec 场景逐条 ----

    #[test]
    fn fresh_db_creates_schema_and_stamps_version_keys() {
        let (_dir, path) = temp_db("fresh.db");
        let (conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();

        assert_eq!(outcome, SyncOutcome::FreshCreated);
        let tables = table_names(&conn);
        assert!(tables.iter().any(|t| t == "alpha"), "缺表 alpha: {tables:?}");
        assert!(tables.iter().any(|t| t == "schema_meta"));
        assert_eq!(meta_get(&conn, "version_format").as_deref(), Some("date"));
        assert_eq!(meta_get(&conn, "version").as_deref(), Some(SCHEMA_VERSION));
        assert!(
            backup_files(_dir.path()).is_empty(),
            "首建不是破坏性迁移，不得产生备份"
        );
    }

    #[test]
    fn second_open_with_matching_structure_is_up_to_date() {
        let (_dir, path) = temp_db("uptodate.db");
        let (_conn, first) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert_eq!(first, SyncOutcome::FreshCreated);
        let (_conn, second) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert_eq!(second, SyncOutcome::UpToDate);
    }

    /// 场景「Missing column is added in place」。
    #[test]
    fn missing_column_is_added_in_place_and_old_rows_stay_readable() {
        let (_dir, path) = temp_db("addcol.db");
        {
            let conn = conn::open(&path).unwrap();
            conn.execute_batch(
                "create table alpha (
                    id    TEXT PRIMARY KEY,
                    name  TEXT NOT NULL,
                    note  TEXT
                );
                create table schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO schema_meta (key, value) VALUES
                    ('version_format', 'date'),
                    ('version', '19990101');",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO alpha (id, name, note) VALUES ('a1', 'old', NULL)",
                [],
            )
            .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert_eq!(
            outcome,
            SyncOutcome::Synced {
                added_columns: 1,
                renamed: 0,
                rebuilt: 0,
                dropped: 0,
            },
            "应原地补一列，不重建、不删库"
        );

        let score: i64 = conn
            .query_row("SELECT score FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(score, 0, "旧行的新列应取 DEFAULT 0");
        assert!(
            backup_files(_dir.path()).is_empty(),
            "纯补列不是破坏性迁移，不得备份"
        );
    }

    /// 场景「Declared rename preserves the column's data」。
    #[test]
    fn declared_rename_preserves_column_data_without_rebuild() {
        let (_dir, path) = temp_db("rename.db");
        {
            let conn = conn::open(&path).unwrap();
            conn.execute_batch(
                "create table alpha (
                    id    TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    note  TEXT,
                    score INTEGER NOT NULL DEFAULT 0
                );
                create table schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO schema_meta (key, value) VALUES ('version_format', 'date');",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO alpha (id, title, note, score) VALUES ('a1', 'kept', 'n', 7)",
                [],
            )
            .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, RENAME_ALPHA_TABLES).unwrap();
        assert_eq!(
            outcome,
            SyncOutcome::Synced {
                added_columns: 0,
                renamed: 1,
                rebuilt: 0,
                dropped: 0,
            },
            "声明改名走 RENAME COLUMN，不重建"
        );

        let names = column_names(&conn, "alpha");
        assert!(names.iter().any(|c| c == "name"), "新列名应在: {names:?}");
        assert!(!names.iter().any(|c| c == "title"), "旧列名应消失: {names:?}");
        let (name, score): (String, i64) = conn
            .query_row("SELECT name, score FROM alpha WHERE id = 'a1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(name, "kept", "改名后每行的值原地保留");
        assert_eq!(score, 7);
        assert!(
            backup_files(_dir.path()).is_empty(),
            "改名不是破坏性迁移，不得备份"
        );
    }

    /// 场景「Undeclared missing-plus-extra pair is not guessed as a rename」。
    #[test]
    fn undeclared_missing_plus_extra_is_dropped_and_added_not_guessed() {
        let (dir, path) = temp_db("pair.db");
        {
            let conn = conn::open(&path).unwrap();
            conn.execute_batch(&format!("{ALPHA_DDL}\n{SCHEMA_META_DDL}"))
                .unwrap();
            stamp_version(&conn).unwrap();
            conn.execute_batch(
                "ALTER TABLE alpha ADD COLUMN stale TEXT;
                 INSERT INTO alpha (id, name, note, score, stale) VALUES ('a1', 'keep', 'n', 7, 'x');",
            )
            .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, PAIR_ALPHA_TABLES).unwrap();
        assert_eq!(
            outcome,
            SyncOutcome::Synced {
                added_columns: 1,
                renamed: 0,
                rebuilt: 0,
                dropped: 1,
            },
            "未声明的一缺一多 = 删 + 加，不猜改名"
        );
        let names = column_names(&conn, "alpha");
        assert!(!names.iter().any(|c| c == "stale"), "多余列应删: {names:?}");
        assert!(names.iter().any(|c| c == "tag"), "缺列应补: {names:?}");
        let (name, tag): (String, Option<String>) = conn
            .query_row("SELECT name, tag FROM alpha WHERE id = 'a1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(name, "keep", "其余行的数据不受影响");
        assert_eq!(tag, None, "补出的可空列取 NULL");
        assert_eq!(backup_files(dir.path()).len(), 1, "删列是破坏性步骤，必须先备份");
    }

    /// 场景「Type change rebuilds the table without losing rows」。
    #[test]
    fn type_change_rebuilds_table_and_keeps_rows_with_affinity_coercion() {
        let (dir, path) = temp_db("typechange.db");
        {
            let conn = conn::open(&path).unwrap();
            conn.execute_batch(
                "create table alpha (
                    id    TEXT PRIMARY KEY,
                    name  INTEGER NOT NULL,
                    note  TEXT,
                    score INTEGER NOT NULL DEFAULT 0
                );
                create index idx_alpha_note on alpha(note);
                create table schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO schema_meta (key, value) VALUES ('version_format', 'date');
                INSERT INTO alpha (id, name, note, score) VALUES ('a1', 42, 'n', 7);",
            )
            .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, ALPHA_INDEXED_TABLES).unwrap();
        assert_eq!(
            outcome,
            SyncOutcome::Synced {
                added_columns: 0,
                renamed: 0,
                rebuilt: 1,
                dropped: 0,
            },
            "类型不符走事务内重建"
        );
        assert_eq!(
            type_affinity(&column_decl(&conn, "alpha", "name")),
            "TEXT",
            "重建后回到 model 的类型"
        );
        let (name, score): (String, i64) = conn
            .query_row("SELECT name, score FROM alpha WHERE id = 'a1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(name, "42", "存量行按列名拷贝（INTEGER 42 经 TEXT 亲和转成 '42'）");
        assert_eq!(score, 7, "其余列数据逐行保全");
        assert!(
            index_names(&conn, "alpha").iter().any(|i| i == "idx_alpha_note"),
            "重建后按注册索引段重建索引"
        );
        assert!(
            !user_table_exists(&conn, &temp_table_name("alpha")).unwrap(),
            "重建后不留临时表"
        );
        // 备份先行：库旁有一份迁移前的完整快照。
        let backups = backup_files(dir.path());
        assert_eq!(backups.len(), 1, "破坏性迁移前必须备份: {backups:?}");
        let snap = conn::open_readonly(&backups[0]).unwrap();
        assert_eq!(
            type_affinity(&column_decl(&snap, "alpha", "name")),
            "INTEGER",
            "备份是迁移前的库"
        );
        let old: i64 = snap
            .query_row("SELECT name FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(old, 42);
    }

    /// 场景「Column removed from the model is dropped」（未被引用的快路径）。
    #[test]
    fn column_removed_from_model_is_dropped_and_data_discarded() {
        let (dir, path) = temp_db("dropcol.db");
        seed_alpha(
            &path,
            "INSERT INTO alpha (id, name, note, score) VALUES ('a1', 'keep', 'note-value', 7);",
        );

        let (conn, outcome) = open_and_sync(&path, DROP_NOTE_TABLES).unwrap();
        assert_eq!(
            outcome,
            SyncOutcome::Synced {
                added_columns: 0,
                renamed: 0,
                rebuilt: 0,
                dropped: 1,
            },
            "无引用的多余列走 DROP COLUMN 快路径"
        );
        let names = column_names(&conn, "alpha");
        assert!(!names.iter().any(|c| c == "note"), "被删列应消失: {names:?}");
        let (name, score): (String, i64) = conn
            .query_row("SELECT name, score FROM alpha WHERE id = 'a1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(name, "keep", "其余列数据存活");
        assert_eq!(score, 7);
        assert_eq!(backup_files(dir.path()).len(), 1, "删列前必须备份");
    }

    /// 场景「Column removed from the model is dropped」的受限形态（索引引用）。
    #[test]
    fn restricted_drop_column_uses_table_rebuild() {
        let (dir, path) = temp_db("dropindexed.db");
        {
            let conn = conn::open(&path).unwrap();
            conn.execute_batch(
                "create table alpha (
                    id    TEXT PRIMARY KEY,
                    name  TEXT NOT NULL,
                    note  TEXT,
                    score INTEGER NOT NULL DEFAULT 0
                );
                create index idx_alpha_note on alpha(note);
                create table schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO schema_meta (key, value) VALUES ('version_format', 'date');
                INSERT INTO alpha (id, name, note, score) VALUES ('a1', 'keep', 'n', 7);",
            )
            .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, DROP_NOTE_TABLES).unwrap();
        assert_eq!(
            outcome,
            SyncOutcome::Synced {
                added_columns: 0,
                renamed: 0,
                rebuilt: 1,
                dropped: 1,
            },
            "被索引引用的多余列升级为重建"
        );
        let names = column_names(&conn, "alpha");
        assert!(!names.iter().any(|c| c == "note"), "被删列应消失: {names:?}");
        let (name, score): (String, i64) = conn
            .query_row("SELECT name, score FROM alpha WHERE id = 'a1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(name, "keep", "重建按列名交集保住其余列");
        assert_eq!(score, 7);
        assert!(
            !index_names(&conn, "alpha").iter().any(|i| i == "idx_alpha_note"),
            "随旧表丢弃的索引不复活（注册清单里没有它）"
        );
        assert_eq!(backup_files(dir.path()).len(), 1);
    }

    /// 场景「Fresh database…」的缺表半边：整表缺失 → 建表，别的表不碰。
    #[test]
    fn missing_table_is_created_in_place_without_touching_others() {
        let (_dir, path) = temp_db("missingtable.db");
        {
            let conn = conn::open(&path).unwrap();
            conn.execute_batch(
                "create table beta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
                 INSERT INTO beta (k, v) VALUES ('b1', 'keep');",
            )
            .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, TWO_TABLE_REGISTRY).unwrap();
        assert_eq!(
            outcome,
            SyncOutcome::Synced {
                added_columns: 0,
                renamed: 0,
                rebuilt: 0,
                dropped: 0,
            },
            "缺表只建表（不计入重建/补列计数）"
        );
        assert!(table_names(&conn).iter().any(|t| t == "alpha"));
        let v: String = conn
            .query_row("SELECT v FROM beta WHERE k = 'b1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, "keep", "未漂移的表不受任何影响");
    }

    /// 场景「Unknown version metadata reconciles by structure」。
    #[test]
    fn unknown_version_format_reconciles_by_structure_without_reset() {
        let (dir, path) = temp_db("badformat.db");
        {
            let (conn, _) = open_and_sync(&path, TEST_TABLES).unwrap();
            conn.execute(
                "INSERT INTO alpha (id, name) VALUES ('a1', 'keep')",
                [],
            )
            .unwrap();
            meta_set(&conn, "version_format", "semver");
        }

        let (conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert_eq!(outcome, SyncOutcome::UpToDate, "结构一致 → 仅补 meta 键");
        assert_eq!(
            meta_get(&conn, "version_format").as_deref(),
            Some("date"),
            "未知格式被改写为当前格式"
        );
        let name: String = conn
            .query_row("SELECT name FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "keep", "数据原样保留");
        assert!(reset_files(dir.path()).is_empty(), "不得有隔离/重置产物");
        assert!(backup_files(dir.path()).is_empty(), "结构未变 → 不备份");
    }

    /// 场景「Unknown version metadata…」的旧迁移链半边（无 schema_meta + user_version）。
    #[test]
    fn legacy_migration_chain_db_is_reconciled_and_keeps_data() {
        let (dir, path) = temp_db("legacy.db");
        {
            // 旧迁移链的库: 只有 user_version, 无 schema_meta; alpha 少一列。
            let conn = conn::open(&path).unwrap();
            conn.execute_batch(
                "create table alpha (
                    id    TEXT PRIMARY KEY,
                    name  TEXT NOT NULL,
                    note  TEXT
                );
                create table legacy_junk (x TEXT);
                INSERT INTO legacy_junk (x) VALUES ('untouched');
                INSERT INTO alpha (id, name, note) VALUES ('a1', 'keep', 'n');",
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 2i64).unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert_eq!(
            outcome,
            SyncOutcome::Synced {
                added_columns: 1,
                renamed: 0,
                rebuilt: 0,
                dropped: 0,
            },
            "旧迁移链库首开被结构 reconcile 吸纳（补列），没有任何重置"
        );
        assert_eq!(meta_get(&conn, "version_format").as_deref(), Some("date"));
        assert_eq!(meta_get(&conn, "version").as_deref(), Some(SCHEMA_VERSION));
        let name: String = conn
            .query_row("SELECT name FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "keep", "存量业务数据保留");
        let score: i64 = conn
            .query_row("SELECT score FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(score, 0, "补出的列取常量默认值");
        assert!(
            table_names(&conn).iter().any(|t| t == "legacy_junk"),
            "未注册的旧表不在迁移范围（不是删表路径）"
        );
        assert!(reset_files(dir.path()).is_empty());
    }

    /// 场景「Version value alone never triggers action」。
    #[test]
    fn version_value_differs_but_structure_matches_never_migrates() {
        let (dir, path) = temp_db("oldvalue.db");
        {
            let (conn, _) = open_and_sync(&path, TEST_TABLES).unwrap();
            meta_set(&conn, "version", "19990101");
            conn.execute(
                "INSERT INTO alpha (id, name) VALUES ('a1', 'keepme')",
                [],
            )
            .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert_eq!(outcome, SyncOutcome::UpToDate, "结构一致时版本值不同绝不动作");
        let value: String = conn
            .query_row("SELECT name FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(value, "keepme", "数据应原样保留");
        assert_eq!(meta_get(&conn, "version").as_deref(), Some(SCHEMA_VERSION));
        assert!(backup_files(dir.path()).is_empty());
        assert!(reset_files(dir.path()).is_empty());
    }

    /// 不可迁移的缺列（非空无默认）→ fail-closed，库一个字节都不动。
    #[test]
    fn unaddable_missing_column_refuses_startup_without_touching_db() {
        let (_dir, path) = temp_db("unaddable.db");
        seed_alpha(&path, "INSERT INTO alpha (id, name, note, score) VALUES ('a1', 'keep', NULL, 7);");
        let before = snapshot(&path);

        let err = open_and_sync(&path, UNADDABLE_ALPHA_TABLES)
            .err()
            .expect("非空无默认的缺列必须拒启动");
        assert!(err.contains("req") && err.contains("非空"), "诊断要点名表列: {err}");
        assert_eq!(snapshot(&path), before, "拒启动时数据库字节与 mtime 都不变");
    }

    /// 场景「Backup failure blocks the destructive migration」。
    #[test]
    fn backup_failure_blocks_destructive_migration() {
        let (_dir, path) = temp_db("backupfail.db");
        seed_alpha(&path, "INSERT INTO alpha (id, name, note, score) VALUES ('a1', 'keep', 'n', 7);");
        // 让备份目标无法写入：同名位置放一个目录（删除/覆盖都会失败）。
        std::fs::create_dir(db_sidecar_path(&path, BACKUP_SUFFIX)).unwrap();
        let before = snapshot(&path);

        let err = open_and_sync(&path, DROP_NOTE_TABLES)
            .err()
            .expect("备份失败必须阻断破坏性迁移");
        assert!(
            err.contains(BACKUP_SUFFIX) && err.contains("备份失败"),
            "诊断要点名备份失败: {err}"
        );
        assert_eq!(snapshot(&path), before, "拒绝前不得动库");
        // 迁移确实没跑：被删的列还在。
        {
            let conn = conn::open_readonly(&path).unwrap();
            assert!(
                column_names(&conn, "alpha").iter().any(|c| c == "note"),
                "破坏性迁移不得执行"
            );
        }
    }

    /// 场景「Failed migration rolls back and refuses startup」：注入数据拷贝
    /// 失败（存量行违反注册 DDL 的 CHECK），事务回滚 + 拒启动 + 库逐字节不变。
    #[test]
    fn failed_migration_rolls_back_and_leaves_db_byte_identical() {
        let (dir, path) = temp_db("failmid.db");
        {
            // 旧表无 CHECK、`name` 是 INTEGER；注册 DDL 的 `name` 是 TEXT（类型
            // 不符 → 重建）且带 `score >= 0` 的 CHECK，存量行 score = -5 拷贝时
            // 违反约束 → 事务中途失败。
            let conn = conn::open(&path).unwrap();
            conn.execute_batch(
                "create table alpha (
                    id    TEXT PRIMARY KEY,
                    name  INTEGER NOT NULL,
                    note  TEXT,
                    score INTEGER NOT NULL DEFAULT 0
                );
                create table schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO schema_meta (key, value) VALUES ('version_format', 'date');
                INSERT INTO alpha (id, name, note, score) VALUES ('a1', 1, 'n', -5);",
            )
            .unwrap();
        }
        let before = snapshot(&path);

        let err = open_and_sync(&path, CHECKED_ALPHA_TABLES)
            .err()
            .expect("中途失败必须拒启动");
        assert!(
            err.contains("1 步失败") && err.contains("重建表"),
            "诊断要点名失败步骤: {err}"
        );
        assert!(
            err.contains("拷贝存量行"),
            "诊断要指出失败在数据拷贝: {err}"
        );
        assert_eq!(snapshot(&path), before, "事务回滚后库必须逐字节不变");
        assert!(reset_files(dir.path()).is_empty(), "绝不回退成删库/隔离");
        {
            let conn = conn::open_readonly(&path).unwrap();
            assert_eq!(
                type_affinity(&column_decl(&conn, "alpha", "name")),
                "INTEGER",
                "回滚后旧结构仍在"
            );
            let score: i64 = conn
                .query_row("SELECT score FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(score, -5, "回滚后旧行原样可见");
            assert!(
                !user_table_exists(&conn, &temp_table_name("alpha")).unwrap(),
                "回滚不留临时表"
            );
        }
    }

    /// 孤儿临时表（上一轮未完成事务的兜底现场）先清理，别的表不受牵连。
    #[test]
    fn orphan_temp_table_is_dropped_before_reconcile() {
        let (dir, path) = temp_db("orphan.db");
        seed_alpha(&path, "INSERT INTO alpha (id, name) VALUES ('a1', 'keep');");
        {
            let conn = conn::open(&path).unwrap();
            conn.execute_batch("create table alpha__sync_tmp (id TEXT);")
                .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert_eq!(
            outcome,
            SyncOutcome::Synced {
                added_columns: 0,
                renamed: 0,
                rebuilt: 0,
                dropped: 0,
            },
            "只剩孤儿表清理，结构本身一致"
        );
        assert!(
            !user_table_exists(&conn, "alpha__sync_tmp").unwrap(),
            "孤儿临时表应被清掉"
        );
        let name: String = conn
            .query_row("SELECT name FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "keep");
        assert_eq!(
            backup_files(dir.path()).len(),
            1,
            "表级清理前也先备份（保守）"
        );
    }

    /// 单表重建不牵连同库的其它表。
    #[test]
    fn rebuild_of_one_table_leaves_the_other_untouched() {
        let (_dir, path) = temp_db("twotables.db");
        {
            let conn = conn::open(&path).unwrap();
            conn.execute_batch(
                "create table alpha (
                    id    TEXT PRIMARY KEY,
                    name  INTEGER NOT NULL,
                    note  TEXT,
                    score INTEGER NOT NULL DEFAULT 0
                );
                create table beta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
                INSERT INTO beta (k, v) VALUES ('b1', 'keep');
                create table schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO schema_meta (key, value) VALUES ('version_format', 'date');
                INSERT INTO alpha (id, name) VALUES ('a1', 1);",
            )
            .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, TWO_TABLE_REGISTRY).unwrap();
        assert_eq!(
            outcome,
            SyncOutcome::Synced {
                added_columns: 0,
                renamed: 0,
                rebuilt: 1,
                dropped: 0,
            }
        );
        let v: String = conn
            .query_row("SELECT v FROM beta WHERE k = 'b1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, "keep");
        assert_eq!(column_names(&conn, "beta"), vec!["k", "v"]);
    }

    /// 迁移日志点名表/列/触发点，并如实记录未知版本元数据被 reconcile。
    #[test]
    fn migration_logs_name_tables_columns_and_triggers() {
        let buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>> = Default::default();
        {
            let captured = SharedBuf(buf.clone());
            let subscriber = tracing_subscriber::fmt()
                .with_ansi(false)
                .with_max_level(tracing::Level::INFO)
                .with_writer(move || captured.clone())
                .finish();
            // 必须全局安装（进程内只此一处）：scoped dispatcher 不参与 callsite
            // interest 的全局缓存；全局 subscriber 注册时重建全部 callsite 的
            // interest，捕获因而是确定的。并行测试可能同样写进缓冲，但断言只做
            // contains，互不干扰。
            tracing::subscriber::set_global_default(subscriber)
                .expect("进程内只允许这一处全局 subscriber");

            // (a) 未知版本格式 + 结构一致
            let (_d1, p1) = temp_db("log-unknown.db");
            {
                let (conn, _) = open_and_sync(&p1, TEST_TABLES).unwrap();
                meta_set(&conn, "version_format", "semver");
            }
            let _ = open_and_sync(&p1, TEST_TABLES).unwrap();

            // (b) 声明改名
            let (_d2, p2) = temp_db("log-rename.db");
            {
                let conn = conn::open(&p2).unwrap();
                conn.execute_batch(
                    "create table alpha (id TEXT PRIMARY KEY, title TEXT NOT NULL, note TEXT,
                     score INTEGER NOT NULL DEFAULT 0);
                     create table schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     INSERT INTO schema_meta (key, value) VALUES ('version_format', 'date');",
                )
                .unwrap();
            }
            let _ = open_and_sync(&p2, RENAME_ALPHA_TABLES).unwrap();

            // (c) 未声明的一缺一多（删 + 加）
            let (_d3, p3) = temp_db("log-pair.db");
            {
                let (conn, _) = open_and_sync(&p3, TEST_TABLES).unwrap();
                conn.execute_batch("ALTER TABLE alpha ADD COLUMN stale TEXT;")
                    .unwrap();
            }
            let _ = open_and_sync(&p3, PAIR_ALPHA_TABLES).unwrap();

            // (d) 类型不符 → 重建
            let (_d4, p4) = temp_db("log-rebuild.db");
            {
                let conn = conn::open(&p4).unwrap();
                conn.execute_batch(
                    "create table alpha (id TEXT PRIMARY KEY, name INTEGER NOT NULL, note TEXT,
                     score INTEGER NOT NULL DEFAULT 0);
                     create table schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     INSERT INTO schema_meta (key, value) VALUES ('version_format', 'date');
                     INSERT INTO alpha (id, name) VALUES ('a1', 1);",
                )
                .unwrap();
            }
            let _ = open_and_sync(&p4, ALPHA_INDEXED_TABLES).unwrap();

            // (e) 受限删列
            let (_d5, p5) = temp_db("log-restricted.db");
            {
                let conn = conn::open(&p5).unwrap();
                conn.execute_batch(
                    "create table alpha (id TEXT PRIMARY KEY, name TEXT NOT NULL, note TEXT,
                     score INTEGER NOT NULL DEFAULT 0);
                     create index idx_alpha_note on alpha(note);
                     create table schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     INSERT INTO schema_meta (key, value) VALUES ('version_format', 'date');",
                )
                .unwrap();
            }
            let _ = open_and_sync(&p5, DROP_NOTE_TABLES).unwrap();

            // (f) 改名来源缺失 → 退化为普通补列（WARN 说明）
            let (_d6, p6) = temp_db("log-renamemiss.db");
            {
                let conn = conn::open(&p6).unwrap();
                conn.execute_batch(
                    "create table alpha (id TEXT PRIMARY KEY, name TEXT NOT NULL,
                     score INTEGER NOT NULL DEFAULT 0);
                     create table schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     INSERT INTO schema_meta (key, value) VALUES ('version_format', 'date');",
                )
                .unwrap();
            }
            let _ = open_and_sync(&p6, RENAME_MISS_TABLES).unwrap();
        }

        let logs = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            logs.contains("version_format") && logs.contains("reconcile"),
            "未知版本格式要 WARN 说明照常 reconcile: {logs}"
        );
        assert!(
            logs.contains("title") && logs.contains("name") && logs.contains("改名"),
            "改名日志要点名表/旧列/新列: {logs}"
        );
        assert!(
            logs.contains("stale") && logs.contains("tag"),
            "未声明的一缺一多要在日志里同时点名两个列: {logs}"
        );
        assert!(
            logs.contains("类型不符") && logs.contains("name"),
            "重建日志要点名类型不符的列: {logs}"
        );
        assert!(
            logs.contains("idx_alpha_note") || logs.contains("重建表"),
            "受限删列的日志要点明重建原因: {logs}"
        );
        assert!(
            logs.contains("退化为普通补列") && logs.contains("old_note"),
            "改名来源缺失要 WARN 说明退化为补列: {logs}"
        );
        assert!(
            logs.contains(BACKUP_SUFFIX),
            "破坏性迁移日志要给出备份落点: {logs}"
        );
    }

    #[test]
    fn corrupt_db_refuses_to_open_and_file_is_untouched() {
        let (_dir, path) = temp_db("corrupt.db");
        let garbage = b"this is definitely not a sqlite database".to_vec();
        std::fs::write(&path, &garbage).unwrap();

        let err = open_and_sync(&path, TEST_TABLES).err().expect("损坏库必须拒启");
        assert!(
            err.contains("损坏") || err.contains("打开状态库失败"),
            "报错要说明损坏: {err}"
        );
        assert!(
            err.contains(path.file_name().unwrap().to_str().unwrap()),
            "报错要含路径: {err}"
        );

        let after = std::fs::read(&path).unwrap();
        assert_eq!(
            after, garbage,
            "损坏库文件一个字节都不能动（同步只处理结构漂移）"
        );
    }

    /// 损坏路径绝不产生备份/隔离产物。
    #[test]
    fn corrupt_db_refusal_touches_nothing() {
        let (dir, path) = temp_db("corrupt2.db");
        std::fs::write(&path, b"this is definitely not a sqlite database").unwrap();

        assert!(
            open_and_sync(&path, TEST_TABLES).is_err(),
            "损坏库必须拒启"
        );
        assert!(reset_files(dir.path()).is_empty(), "不得有隔离产物");
        assert!(backup_files(dir.path()).is_empty(), "不得有备份产物");
    }

    /// 3.1：计划构建是**只读**的，且动作序列逐项有序（改名 → 补列 → 删列）。
    #[test]
    fn plan_builds_ordered_action_sequence_read_only() {
        let (_dir, path) = temp_db("plan.db");
        {
            // 旧结构：`title`（model 声明由它改名成 `name`）、多一列 `stale`、
            // 少一列 `tag`；没有类型漂移 → 走增量快路径而非重建。
            let conn = conn::open(&path).unwrap();
            conn.execute_batch(
                "create table alpha (
                    id    TEXT PRIMARY KEY,
                    title TEXT,
                    note  TEXT,
                    score INTEGER NOT NULL DEFAULT 0,
                    stale TEXT
                );
                create table schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO schema_meta (key, value) VALUES ('version_format', 'date');
                INSERT INTO alpha (id, title, score) VALUES ('a1', 'kept', 7);",
            )
            .unwrap();
        }
        let before = std::fs::read(&path).unwrap();

        let conn = conn::open_readonly(&path).unwrap();
        let plan = build_plan(&conn, PLAN_ALPHA_TABLES, false).unwrap();

        let labels: Vec<String> = plan.actions.iter().map(|a| a.label()).collect();
        assert_eq!(
            labels,
            vec![
                "改名 alpha.title -> name",
                "补列 alpha.tag",
                "删列 alpha.stale"
            ],
            "动作序列必须按 改名 → 补列 → 删列 排列"
        );
        assert_eq!(
            (
                plan.added_columns,
                plan.renamed,
                plan.rebuilt,
                plan.dropped
            ),
            (1, 1, 0, 1),
            "结构化计数与动作序列一致"
        );
        assert!(plan.destructive, "含删列 → 破坏性（执行前必须备份）");
        drop(conn);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "计划构建阶段不得写库（只读）"
        );
    }

    /// 3.2：声明的改名来源列不存在（新装库 / 已迁过）→ 退化为普通补列 + WARN，
    /// 绝不猜、也不报错（该列可空时补得进来）。
    #[test]
    fn declared_rename_with_missing_source_degrades_to_plain_add() {
        let (_dir, path) = temp_db("renamemiss.db");
        {
            let conn = conn::open(&path).unwrap();
            conn.execute_batch(
                "create table alpha (
                    id    TEXT PRIMARY KEY,
                    name  TEXT NOT NULL,
                    score INTEGER NOT NULL DEFAULT 0
                );
                create table schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO schema_meta (key, value) VALUES ('version_format', 'date');
                INSERT INTO alpha (id, name, score) VALUES ('a1', 'keep', 7);",
            )
            .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, RENAME_MISS_TABLES).unwrap();
        assert_eq!(
            outcome,
            SyncOutcome::Synced {
                added_columns: 1,
                renamed: 0,
                rebuilt: 0,
                dropped: 0,
            },
            "来源列不存在 → 只补列，不改名、不重建"
        );
        assert!(
            column_names(&conn, "alpha").iter().any(|c| c == "note"),
            "旧列名 `old_note` 与新列名都不在 → 按缺列补出 `note`"
        );
        let name: String = conn
            .query_row("SELECT name FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "keep", "存量行不受影响");
    }

    // ---- 原语 ----

    #[test]
    fn create_table_ddl_with_name_replaces_only_the_registered_table_name() {
        let ddl = "CREATE TABLE alpha (
            id   TEXT PRIMARY KEY,   -- alpha 的注释里也出现表名
            note TEXT
        );";
        let rebuilt = create_table_ddl_with_name(ddl, "alpha", "alpha__sync_tmp").unwrap();
        assert!(
            rebuilt.starts_with("CREATE TABLE alpha__sync_tmp ("),
            "{rebuilt}"
        );
        assert!(
            rebuilt.contains("-- alpha 的注释里也出现表名"),
            "注释里的同名子串不得被替换: {rebuilt}"
        );
        assert_eq!(rebuilt.matches("alpha__sync_tmp").count(), 1);
    }

    #[test]
    fn create_table_ddl_with_name_supports_if_not_exists_and_rejects_mismatch() {
        let ddl = "CREATE TABLE IF NOT EXISTS usage_records (id INTEGER PRIMARY KEY);";
        let rebuilt =
            create_table_ddl_with_name(ddl, "usage_records", "usage_records__sync_tmp").unwrap();
        assert_eq!(
            rebuilt,
            "CREATE TABLE IF NOT EXISTS usage_records__sync_tmp (id INTEGER PRIMARY KEY);"
        );

        let err = create_table_ddl_with_name(ddl, "other_table", "x").expect_err("名字不符应报错");
        assert!(err.contains("usage_records") && err.contains("other_table"), "{err}");
    }

    #[test]
    fn type_affinity_follows_sqlite_rules() {
        assert_eq!(type_affinity("INTEGER"), "INTEGER");
        assert_eq!(type_affinity("BIGINT"), "INTEGER");
        assert_eq!(type_affinity("VARCHAR(255)"), "TEXT");
        assert_eq!(type_affinity("TEXT"), "TEXT");
        assert_eq!(type_affinity(""), "BLOB");
        assert_eq!(type_affinity("BLOB"), "BLOB");
        assert_eq!(type_affinity("DOUBLE"), "REAL");
        assert_eq!(type_affinity("DECIMAL(10,5)"), "NUMERIC");
    }

    #[test]
    fn add_column_sql_includes_not_null_and_default() {
        let col = SchemaColumn {
            name: "flag",
            affinity: "INTEGER",
            default: Some("0"),
            not_null: true,
            rename_from: None,
        };
        assert_eq!(
            add_column_sql("alpha", &col),
            "ALTER TABLE \"alpha\" ADD COLUMN \"flag\" INTEGER NOT NULL DEFAULT 0"
        );
        let nullable = SchemaColumn {
            name: "note",
            affinity: "TEXT",
            default: None,
            not_null: false,
            rename_from: Some("old_note"),
        };
        assert_eq!(
            add_column_sql("alpha", &nullable),
            "ALTER TABLE \"alpha\" ADD COLUMN \"note\" TEXT",
            "rename_from 不参与补列 SQL"
        );
    }

    #[test]
    fn effective_name_maps_renamed_live_column() {
        let live = LiveColumn {
            name: "title".to_string(),
            decl: "TEXT".to_string(),
        };
        static COL: SchemaColumn = SchemaColumn {
            name: "name",
            affinity: "TEXT",
            default: None,
            not_null: true,
            rename_from: Some("title"),
        };
        let renames = vec![("title".to_string(), &COL)];
        assert_eq!(effective_name(&live, &renames), "name");
        let plain = LiveColumn {
            name: "score".to_string(),
            decl: "INTEGER".to_string(),
        };
        assert_eq!(effective_name(&plain, &renames), "score");
    }
}
