//! `SessionCheckpointRow` — session_checkpoints 表的 ActiveRecord struct + 域查询。
//!
//! session-transcript-durability 1.1：会话面（转录条目 / 泊车审批 / 会话级
//! usage）的周期 checkpoint 落盘——**一会话一行快照 blob**（change design
//! D1）：行 = (`session_id` 主键, `updated_at`, `transcript_json`,
//! `parked_json`, usage 标量三列)。单行 upsert（生成的 `save()`）即单事务
//! 原子替换——崩溃时留下的只能是完整旧快照或完整新快照；启动回放 =
//! [`load_session_checkpoints`] 全表扫一遍。
//!
//! 关联域类型（`TurnEntry` / 泊车登记）**不进本 crate**（sebas-models 不依赖
//! sebas-dispatch / sebas-domain 的会话面类型）：JSON blob 由写入方（dispatch
//! 引擎）序列化、回放方反序列化，这里只承载不透明文本与 usage 标量——
//! struct 仍即表结构事实源，约束（PRIMARY KEY）只在根 crate 注册表的 DDL 里。

use sebas_db::rusqlite::Connection;
use sebas_schema_derive::{ActiveRecord, SchemaColumns};

/// session_checkpoints 表行：一行 = 一个会话的最近 checkpoint 快照。
#[derive(Debug, Clone, PartialEq, Eq, SchemaColumns, ActiveRecord)]
#[active_record(table = "session_checkpoints")]
#[active_record(pk = "session_id")]
pub struct SessionCheckpointRow {
    /// 转录寻址 id（= 引擎侧 session_id，即映射的 `transcript_id`）。
    pub session_id: String,
    /// 快照写入时刻（unix 秒；诊断/排序用，不参与回放语义）。
    pub updated_at: i64,
    /// 转录条目 JSON 数组（写入方 `Vec<TurnEntry>` 序列化产物）。
    pub transcript_json: String,
    /// 泊车审批 JSON 数组（写入方泊车登记序列化产物；空泊车 = `[]`）。
    pub parked_json: String,
    /// 会话累计输入 token。
    pub usage_in: i64,
    /// 会话累计输出 token。
    pub usage_out: i64,
    /// 会话是否上报过任何 token 计数（区分「未上报」与「已上报 0」——
    /// 快照投影门控位，原样随行往返）。
    pub usage_reported: bool,
}

/// 加载全部 checkpoint 行（启动回放源；无排序语义——回放方按会话寻址）。
pub fn load_session_checkpoints(conn: &mut Connection) -> Result<Vec<SessionCheckpointRow>, String> {
    SessionCheckpointRow::all(conn).map_err(|e| format!("查询 session_checkpoints 失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> SessionCheckpointRow {
        SessionCheckpointRow {
            session_id: "sess-1".into(),
            updated_at: 42,
            transcript_json: r#"[{"position":0}]"#.into(),
            parked_json: "[]".into(),
            usage_in: 10,
            usage_out: 25,
            usage_reported: true,
        }
    }

    /// 列元数据形状钉：列名/亲和/默认值/可空性与注册表 DDL 逐列对应
    /// （sqlite-auto-schema-sync 以注册列 diff，非 DDL 文本）。
    #[test]
    fn schema_columns_match_target_shape() {
        use sebas_db::record::Record;
        use sebas_db::schema::SchemaColumn;
        assert_eq!(SessionCheckpointRow::TABLE, "session_checkpoints");
        assert_eq!(SessionCheckpointRow::PK_COLUMNS, &["session_id"]);
        let baseline: &[SchemaColumn] = &[
            SchemaColumn { name: "session_id", affinity: "TEXT", default: None, not_null: true, rename_from: None },
            SchemaColumn { name: "updated_at", affinity: "INTEGER", default: None, not_null: true, rename_from: None },
            SchemaColumn { name: "transcript_json", affinity: "TEXT", default: None, not_null: true, rename_from: None },
            SchemaColumn { name: "parked_json", affinity: "TEXT", default: None, not_null: true, rename_from: None },
            SchemaColumn { name: "usage_in", affinity: "INTEGER", default: None, not_null: true, rename_from: None },
            SchemaColumn { name: "usage_out", affinity: "INTEGER", default: None, not_null: true, rename_from: None },
            SchemaColumn { name: "usage_reported", affinity: "INTEGER", default: None, not_null: true, rename_from: None },
        ];
        assert_eq!(SessionCheckpointRow::schema_columns(), baseline);
        assert_eq!(
            SessionCheckpointRow::COLUMNS,
            baseline.iter().map(|c| c.name).collect::<Vec<_>>().as_slice()
        );
    }

    /// 建/读行为**不在本 crate 测**：那需要一张 `session_checkpoints` 表，
    /// 而手写 `CREATE TABLE` 只允许在根 crate 注册表（AGENTS.md 持久层准入
    /// 规则 1）。往返覆盖在根 crate `tests/`（checkpoint 表单测）与 dispatch
    /// 引擎单测（写出行内容与内存态一致；回放后 turns() 与 checkpoint 一致）。
    #[test]
    fn row_is_cloneable_and_comparable() {
        let a = row();
        let mut b = a.clone();
        assert_eq!(a, b);
        b.usage_in += 1;
        assert_ne!(a, b);
    }
}
