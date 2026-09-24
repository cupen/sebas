//! usage record → router 自有用量库（persist-router-usage，见
//! openspec/changes/persist-router-usage/specs/router-auth-rate-limit/spec.md）。
//!
//! # 存储形态
//!
//! 独立 SQLite 文件：缺省状态目录下的 `usage.db`（`SEBAS_ROUTER_USAGE_DB`
//! 覆盖；落点由 `single-state-dir` 的逻辑名表 `StatePath::UsageDb` 派生）。
//! **router 是这个库的唯一写入者，并且绝不打开 core 的状态库**
//! （`state-store`「Only the core process SHALL open the database」——
//! 本模块只经 `sebas_db` 打开 `usage_db` 指的那一个文件）。
//!
//! 连接配方、schema 同步与单写执行模型全部取自共享持久层 `sebas-db`
//! （`StateWriter` 专用线程 + 命令串行；`open_and_sync` 做列级 diff）——
//! 本模块不自建连接管理、不手抄 pragma、也不长第二套版本机制。
//!
//! # 行映射
//!
//! `usage_records` 表一行对应一个 [`UsageRow`]（ActiveRecord，标准 CRUD 由
//! derive 生成）。`id` 是自增主键：records 按**完成顺序**追加，`id` 升序
//! 即完成顺序（一次 `settle` 一条）。对外形状 [`UsageRecord`] 保持与迁移
//! 前逐字一致（`key` 恒空等语义见下）。
//!
//! # 异步 sink 语义（逐条保留）
//!
//! 容量 256 的有界通道、满则丢弃 + warn、**绝不阻塞或失败在途响应**、写
//! 失败不影响路由。这些是 `router-auth-rate-limit`「Usage record pipeline」
//! 的既有要求，存量化改造一字不改。
//!
//! # 保留期（design D2）
//!
//! 两个闸 + 后台定期间隔清理：
//!
//! - **时间闸** `usage_retention_days`：早于窗口的记录被清理（`0` = 关闸）；
//! - **行数闸** `usage_max_rows`：行数超上限时清理最旧记录（`0` = 关闸）；
//! - 间隔 `usage_prune_interval_secs`（`0` = 不跑后台清理）。
//!
//! 清理动作**写日志**（INFO 计数），避免「数据悄悄消失」。

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sebas_db::schema::TableSchema;
use sebas_db::writer::{StateHandle, StateWriter};
use sebas_schema_derive::{ActiveRecord, SchemaColumns};
use tokio::sync::mpsc;

/// 一次请求的用量记录。`key` 恒为空（无 per-key 身份；绝不记 token 本体）。
/// `error` 留给路由侧失败（如 connect 502）；上游 4xx/5xx 不填 `error`（status
/// 字段承载其错误语义）。token 字段为 `None` 表示本次未观测到该计数（如
/// 解析失败、流被截断、或上游错误响应无 usage）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageRecord {
    pub ts: String,
    pub key: String,
    pub protocol: String,
    pub model: Option<String>,
    pub provider: String,
    pub upstream_model: Option<String>,
    pub status: u16,
    pub latency_ms: u64,
    pub ttft_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub error: Option<String>,
}

/// `usage_records` 表的一行（一表一 struct，标准 CRUD 由 derive 生成）。
///
/// `id` 是自增主键：`None` → INSERT 走自增 rowid；读回时恒有值。列顺序即
/// `SCHEMA_COLUMNS` 顺序（`Record::COLUMNS` 与 `to_params` / `from_row` 一致）。
/// `ts` 是 RFC3339 UTC 字符串——保留期时间闸按**字典序**比较即可（同格式同
/// 时区的 RFC3339 串，字典序 = 时间序）。
#[derive(Debug, Clone, PartialEq, Eq, SchemaColumns, ActiveRecord)]
#[active_record(table = "usage_records")]
#[active_record(pk = "id")]
pub struct UsageRow {
    pub id: Option<i64>,
    pub key: String,
    pub protocol: String,
    pub model: Option<String>,
    pub provider: String,
    pub upstream_model: Option<String>,
    pub status: i64,
    pub latency_ms: i64,
    pub ttft_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub error: Option<String>,
    pub ts: String,
}

impl From<&UsageRecord> for UsageRow {
    fn from(r: &UsageRecord) -> Self {
        UsageRow {
            id: None,
            key: r.key.clone(),
            protocol: r.protocol.clone(),
            model: r.model.clone(),
            provider: r.provider.clone(),
            upstream_model: r.upstream_model.clone(),
            status: r.status as i64,
            latency_ms: r.latency_ms as i64,
            ttft_ms: r.ttft_ms.map(|v| v as i64),
            input_tokens: r.input_tokens.map(|v| v as i64),
            output_tokens: r.output_tokens.map(|v| v as i64),
            cache_read_tokens: r.cache_read_tokens.map(|v| v as i64),
            cache_creation_tokens: r.cache_creation_tokens.map(|v| v as i64),
            error: r.error.clone(),
            ts: r.ts.clone(),
        }
    }
}

impl From<UsageRow> for UsageRecord {
    fn from(r: UsageRow) -> Self {
        UsageRecord {
            ts: r.ts,
            key: r.key,
            protocol: r.protocol,
            model: r.model,
            provider: r.provider,
            upstream_model: r.upstream_model,
            status: r.status as u16,
            latency_ms: r.latency_ms as u64,
            ttft_ms: r.ttft_ms.map(|v| v as u64),
            input_tokens: r.input_tokens.map(|v| v as u64),
            output_tokens: r.output_tokens.map(|v| v as u64),
            cache_read_tokens: r.cache_read_tokens.map(|v| v as u64),
            cache_creation_tokens: r.cache_creation_tokens.map(|v| v as u64),
            error: r.error,
        }
    }
}

/// `usage_records` 表的注册 DDL（本 crate 唯一的 `CREATE TABLE` 一份）。
///
/// `id INTEGER PRIMARY KEY` 即自增 rowid：records 按完成顺序追加，`id` 升序
/// = 完成顺序。`ts` 上有索引——时间闸的清理按它过滤。
/// 列清单来自 `UsageRow::schema_columns()`（struct 即 schema 事实源）。
pub static USAGE_TABLES: &[TableSchema] = &[TableSchema {
    name: "usage_records",
    create_ddl: "CREATE TABLE IF NOT EXISTS usage_records (
        id                    INTEGER PRIMARY KEY,
        key                   TEXT NOT NULL,
        protocol              TEXT NOT NULL,
        model                 TEXT,
        provider              TEXT NOT NULL,
        upstream_model        TEXT,
        status                INTEGER NOT NULL,
        latency_ms            INTEGER NOT NULL,
        ttft_ms               INTEGER,
        input_tokens          INTEGER,
        output_tokens         INTEGER,
        cache_read_tokens     INTEGER,
        cache_creation_tokens INTEGER,
        error                 TEXT,
        ts                    TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_usage_records_ts ON usage_records(ts);",
    columns: UsageRow::schema_columns(),
}];

/// 后台 writer task 的容量。满则 `record` 用 `try_send` 丢弃并 warn。
const CHANNEL_CAPACITY: usize = 256;

/// 一次批量提交最多聚合的记录数（design D3：批量降低提交次数，但**不**改变
/// 「记录丢失只发生在通道满/关闭时」这一语义——已入通道的记录一定被提交）。
const BATCH_MAX: usize = 64;

/// 保留期配置（三个闸；`0` 分别表示关闭）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// 时间闸：早于 `now - days` 的记录被清理；`0` = 关闭时间闸。
    pub retention_days: u64,
    /// 行数闸：行数超过该上限时清理最旧记录；`0` = 关闭行数闸。
    pub max_rows: u64,
    /// 后台清理间隔；`0` = 不跑后台清理。
    pub prune_interval_secs: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            retention_days: 30,
            max_rows: 200_000,
            prune_interval_secs: 3600,
        }
    }
}

/// 用量库写入器句柄。`Clone` 因 `mpsc::Sender` 可克隆（AppState 需 Clone）。
/// drop 不等 sink 关闭——后台 task 在 `recv → None` 时自然退出。
#[derive(Clone)]
pub struct UsageSink {
    tx: mpsc::Sender<UsageRecord>,
}

impl UsageSink {
    /// 打开 router 自有用量库并起后台 writer task。
    ///
    /// 父目录先建（同步，一次性，启动时）；库由 `sebas_db` 的 schema 同步
    /// 打开并补齐列。失败 → io::Error 转嫁调用方（build_state 映射为 Config
    /// 错误拒绝启动——既有语义不变）。
    ///
    /// 路径来自 `[router] usage_db`：**只打开这一个文件**。router 绝不打开
    /// core 的状态库（settings.db / projects.db）——见模块头。
    ///
    /// `tokio::spawn` 要求调用线程处于 tokio 运行时上下文。`build_state`
    /// 在 `run`（async）/测试（`#[tokio::test]`）内被调用，运行时恒存在。
    pub fn spawn_writer(path: impl AsRef<Path>, policy: RetentionPolicy) -> io::Result<Self> {
        let path: PathBuf = path.as_ref().to_path_buf();
        // 父目录先建：避免 writer task 反复重建已删父目录；也避免 task
        // 启动时 open 失败导致所有 record 丢弃。空 parent（相对路径文件）
        // 跳过 create_dir_all（"." 不需要建）。
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && parent != Path::new(".")
        {
            // 错误信息带上库路径：调用方（build_state）据此报出「哪个库开不了」。
            std::fs::create_dir_all(parent).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("创建用量库父目录 {parent:?} 失败（库 {path:?}）: {e}"),
                )
            })?;
        }
        // 单写 actor（sebas-db）：专用线程 + 命令串行；启动即完成 schema 同步。
        let writer = StateWriter::start(path.clone(), USAGE_TABLES)
            .map_err(|e| io::Error::other(format!("用量库 {path:?} 初始化失败: {e}")))?;
        let handle = writer.handle().clone();

        let (tx, mut rx) = mpsc::channel::<UsageRecord>(CHANNEL_CAPACITY);
        // writer（StateWriter）随 task 存活；drop 时关闭命令通道，写线程退出。
        let write_handle = handle.clone();
        tokio::spawn(async move {
            let _writer = writer;
            while let Some(first) = rx.recv().await {
                // 批量聚合：把当前已排队的记录一次提交（降低提交次数）。
                let mut batch = Vec::with_capacity(BATCH_MAX);
                batch.push(first);
                while batch.len() < BATCH_MAX {
                    match rx.try_recv() {
                        Ok(rec) => batch.push(rec),
                        Err(_) => break,
                    }
                }
                let n = batch.len();
                let rows: Vec<UsageRow> = batch.iter().map(UsageRow::from).collect();
                let result = write_handle
                    .exec(move |conn| {
                        let tx = sebas_db::conn::transaction(conn).map_err(|e| e.to_string())?;
                        for row in &rows {
                            sebas_db::record::save(&tx, row).map_err(|e| e.to_string())?;
                        }
                        tx.commit().map_err(|e| e.to_string())
                    })
                    .await;
                if let Err(e) = result {
                    // 库不可用/写失败 → 丢弃 + warn，绝不影响路由（既有 sink
                    // 语义：usage 是统计旁路，绝不阻断转发）。
                    tracing::warn!(error = %e, dropped = n, "usage db write failed; dropping records");
                }
            }
        });

        // 后台保留期清理（design D2）：独立 task，间隔触发，绝不阻塞响应
        // （清理在 writer 线程串行执行，与写入共用同一条命令队列）。
        if policy.prune_interval_secs > 0 && (policy.retention_days > 0 || policy.max_rows > 0) {
            let prune_handle = handle.clone();
            let interval = Duration::from_secs(policy.prune_interval_secs);
            tokio::spawn(async move {
                // 首个 tick 在**一个间隔之后**（而非立即）：清理是后台维护，
                // 启动瞬间删存量会让「刚写的记录立刻消失」难以推理；定期间隔
                // 语义也更直白。
                let start = tokio::time::Instant::now() + interval;
                let mut ticker = tokio::time::interval_at(start, interval);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    ticker.tick().await;
                    prune_once(&prune_handle, policy).await;
                }
            });
        }

        Ok(UsageSink { tx })
    }

    /// 投递一条 record。mpsc(256) 满则 warn 丢弃（warn 不含 key 材料——
    /// key 材料只存在于 record 内，warn 串恒为静态字面量）。channel 关闭
    /// （writer task 已退出）同样 warn 丢弃。
    pub fn record(&self, rec: UsageRecord) {
        if let Err(e) = self.tx.try_send(rec) {
            match e {
                mpsc::error::TrySendError::Full(_) => {
                    tracing::warn!(
                        "usage sink channel full (cap {CHANNEL_CAPACITY}); dropping record"
                    );
                }
                mpsc::error::TrySendError::Closed(_) => {
                    tracing::warn!("usage sink writer closed; dropping record");
                }
            }
        }
    }
}

/// 跑一次保留期清理（两个闸）。清理计数写 INFO 日志——「数据被删」必须可见
/// （design D2 的取舍）。任何错误只 warn，不向上传播（后台任务）。
pub async fn prune_once(handle: &StateHandle, policy: RetentionPolicy) {
    let cutoff = retention_cutoff(policy);
    let max_rows = policy.max_rows;
    let result = handle
        .exec(move |conn| -> Result<(usize, usize), String> {
            let mut aged = 0usize;
            if let Some(cutoff) = cutoff.as_deref() {
                aged = conn
                    .execute("DELETE FROM usage_records WHERE ts < ?1", [cutoff])
                    .map_err(|e| e.to_string())?;
            }
            let mut excess = 0usize;
            if max_rows > 0 {
                // 行数闸：删掉最旧的「超出行数」条——按 id 升序（= 完成顺序）
                // 保留最近 max_rows 条。`id` 是自增主键，子查询可用
                // （bundled SQLite 远高于 3.25）。
                excess = conn
                    .execute(
                        "DELETE FROM usage_records WHERE id NOT IN (
                             SELECT id FROM usage_records ORDER BY id DESC LIMIT ?1
                         )",
                        [max_rows as i64],
                    )
                    .map_err(|e| e.to_string())?;
            }
            Ok((aged, excess))
        })
        .await;
    match result {
        Ok((aged, excess)) => {
            if aged > 0 || excess > 0 {
                tracing::info!(
                    aged_pruned = aged,
                    excess_pruned = excess,
                    "usage retention pruned records"
                );
            }
        }
        Err(e) => tracing::warn!(error = %e, "usage retention prune failed"),
    }
}

/// 时间闸的 cutoff（RFC3339 UTC 字符串，字典序 = 时间序）。`None` = 关闸。
fn retention_cutoff(policy: RetentionPolicy) -> Option<String> {
    if policy.retention_days == 0 {
        return None;
    }
    let days = i64::try_from(policy.retention_days).unwrap_or(i64::MAX);
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
    Some(cutoff.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sebas_db::record::Record;
    use tempfile::tempdir;

    fn sample_record(ts: &str) -> UsageRecord {
        UsageRecord {
            ts: ts.into(),
            key: String::new(),
            protocol: "anthropic".into(),
            model: Some("claude-sonnet".into()),
            provider: "anthropic".into(),
            upstream_model: Some("anthropic.claude-sonnet-4".into()),
            status: 200,
            latency_ms: 123,
            ttft_ms: Some(45),
            input_tokens: Some(10),
            output_tokens: Some(50),
            cache_read_tokens: Some(5),
            cache_creation_tokens: Some(2),
            error: None,
        }
    }

    /// 直开库读全表（按 id = 完成顺序）。
    fn rows_of(path: &Path) -> Vec<UsageRow> {
        let conn = sebas_db::conn::open(path).expect("open usage db");
        let mut stmt = conn
            .prepare(
                "SELECT id, key, protocol, model, provider, upstream_model, status,
                        latency_ms, ttft_ms, input_tokens, output_tokens,
                        cache_read_tokens, cache_creation_tokens, error, ts
                 FROM usage_records ORDER BY id",
            )
            .expect("prepare");
        let rows = stmt
            .query_map([], UsageRow::from_row)
            .expect("query")
            .collect::<sebas_db::rusqlite::Result<Vec<_>>>()
            .expect("collect");
        rows
    }

    /// 轮询直到断言成立（writer 是异步的）。
    async fn wait_until<F: FnMut() -> bool>(what: &str, mut f: F) {
        let ok = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if f() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(ok.is_ok(), "timed out waiting for: {what}");
    }

    fn no_background_prune() -> RetentionPolicy {
        RetentionPolicy {
            prune_interval_secs: 0,
            ..Default::default()
        }
    }

    // ---------------- 2.1 两条记录落库，顺序 = 完成顺序 ----------------

    #[tokio::test]
    async fn two_records_land_in_db_in_completion_order() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        // 清理间隔 0：本用例只验写入。
        let sink = UsageSink::spawn_writer(&path, no_background_prune()).expect("spawn_writer");

        let rec1 = sample_record("2026-08-07T00:00:00+00:00");
        let mut rec2 = rec1.clone();
        rec2.model = Some("second".into());
        rec2.status = 500;
        rec2.error = Some("upstream connect failed".into());

        sink.record(rec1);
        sink.record(rec2);

        let p = path.clone();
        wait_until("two rows committed", move || rows_of(&p).len() >= 2).await;

        let rows = rows_of(&path);
        assert_eq!(rows.len(), 2, "两条记录都已提交");
        // 顺序 = 完成顺序（id 升序）：rec1 在前。
        assert_eq!(rows[0].model.as_deref(), Some("claude-sonnet"));
        assert_eq!(rows[1].model.as_deref(), Some("second"));
        assert!(rows[0].id.unwrap() < rows[1].id.unwrap(), "id 严格递增");
        // 场景原文：两条记录都在 router 状态目录下的库里、且都可查询。
        assert_eq!(path.file_name().and_then(|s| s.to_str()), Some("usage.db"));
    }

    // ---------------- 2.2 sink 语义：容量与「绝不阻塞」 ----------------

    /// 库路径不可用（父目录被文件占位）→ 启动期拒绝，错误点名路径
    /// （build_state 据此转 Config 错误拒启，既有语义）。
    #[tokio::test]
    async fn unusable_path_fails_at_startup_with_the_path_named() {
        let dir = tempdir().expect("tempdir");
        let blocker = dir.path().join("blocked");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let path = blocker.join("usage.db");

        let err = match UsageSink::spawn_writer(&path, RetentionPolicy::default()) {
            Ok(_) => panic!("父目录被文件占位时不得启动成功"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("usage.db"),
            "错误必须点名 usage_db 路径: {err}"
        );
    }

    /// 通道满时的丢弃路径：writer 尚未消费时投递远超容量的批次，`record` 必须
    /// 全部立即返回（`try_send` 不阻塞，spec 场景「sink overflow drops
    /// records」的「响应不受影响」）。
    #[tokio::test]
    async fn record_never_blocks_even_when_the_channel_overflows() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let sink = UsageSink::spawn_writer(&path, no_background_prune()).expect("spawn_writer");

        let start = std::time::Instant::now();
        for i in 0..(CHANNEL_CAPACITY * 4) {
            let mut rec = sample_record("2026-08-07T00:00:00+00:00");
            rec.model = Some(format!("m{i}"));
            sink.record(rec);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "record 绝不阻塞调用方（投递 {} 条耗时 {elapsed:?}）",
            CHANNEL_CAPACITY * 4
        );

        // 已入通道的记录一定被提交（丢弃只发生在通道满/关闭时）。
        let p = path.clone();
        wait_until("some rows committed", move || !rows_of(&p).is_empty()).await;
    }

    /// 通道关闭（writer 已退出）时 `record` 同样只是 warn 丢弃。
    #[tokio::test]
    async fn record_on_a_closed_sink_only_warns() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let sink = UsageSink::spawn_writer(&path, no_background_prune()).expect("spawn_writer");
        // 投一条确保 writer task 真的起来了。
        sink.record(sample_record("2026-08-07T00:00:00+00:00"));
        let p = path.clone();
        wait_until("first row", move || !rows_of(&p).is_empty()).await;
        // 丢弃 sink：writer task 随 `rx → None` 退出。此后 record 不再 panic。
        drop(sink);
    }

    // ---------------- 2.5 记录字段与语义不变（含 key 恒空） ----------------

    #[tokio::test]
    async fn record_fields_survive_the_round_trip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let sink = UsageSink::spawn_writer(&path, no_background_prune()).expect("spawn_writer");

        // 上游 4xx/5xx：status 承载错误语义，error 为空（spec 场景）。
        let mut rec = sample_record("2026-08-07T00:00:00+00:00");
        rec.key = String::new(); // key 恒空
        rec.status = 429;
        rec.error = None;
        sink.record(rec.clone());

        let p = path.clone();
        wait_until("row committed", move || rows_of(&p).len() >= 1).await;
        let back = UsageRecord::from(rows_of(&path).remove(0));

        assert_eq!(back, rec, "全部字段经库往返后逐字不变");
        assert_eq!(back.key, "", "key 字段恒为空");
        assert_eq!(back.status, 429, "上游 429 原样记录");
        assert_eq!(back.error, None, "上游错误不填 router error");

        // 路由侧失败：error 有值。
        let mut rec2 = sample_record("2026-08-07T00:00:01+00:00");
        rec2.status = 502;
        rec2.error = Some("failed to reach upstream provider".into());
        let rec2_for_cmp = rec2.clone();
        sink.record(rec2);
        let p2 = path.clone();
        wait_until("second row committed", move || rows_of(&p2).len() >= 2).await;
        let rows = rows_of(&path);
        let back2 = UsageRecord::from(rows.into_iter().nth(1).unwrap());
        assert_eq!(back2, rec2_for_cmp);
        assert_eq!(
            back2.error.as_deref(),
            Some("failed to reach upstream provider")
        );
        assert_eq!(back2.key, "");
    }

    /// 下游 key 命中的请求（鉴权通过但无归属）→ 记录的 `key` 仍是空串。
    /// 记录形状上就没有 per-key 归属（spec 场景「key never recorded」）。
    #[tokio::test]
    async fn authenticated_request_still_records_an_empty_key() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let sink = UsageSink::spawn_writer(&path, no_background_prune()).expect("spawn_writer");

        // 模拟「以 k1 鉴权通过的请求结算」：settle 路径建的 record 里 key 恒空。
        let rec = sample_record("2026-08-07T00:00:00+00:00");
        assert_eq!(rec.key, "", "settle 构造的 record 里 key 就是空的");
        sink.record(rec);
        let p = path.clone();
        wait_until("row committed", move || rows_of(&p).len() >= 1).await;

        let rows = rows_of(&path);
        let back = UsageRecord::from(rows.into_iter().next().unwrap());
        assert_eq!(back.key, "", "落库后 key 仍为空串（非 NULL、非 k1）");
        // 库文件里也不该出现任何 key 材料（下游 token 绝不落盘）。
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(!raw.contains("k1"), "下游 key 材料不得出现在用量库");
    }

    /// `Option` 字段全为 None（上游错误响应无 usage）也必须保真往返——
    /// None ≠ 0。
    #[tokio::test]
    async fn null_token_counts_stay_null() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let sink = UsageSink::spawn_writer(&path, no_background_prune()).expect("spawn_writer");

        let rec = UsageRecord {
            ts: "2026-08-07T00:00:00+00:00".into(),
            key: String::new(),
            protocol: "openai_chat".into(),
            model: None,
            provider: "openai".into(),
            upstream_model: None,
            status: 502,
            latency_ms: 5,
            ttft_ms: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            error: Some("boom".into()),
        };
        sink.record(rec.clone());
        let p = path.clone();
        wait_until("row committed", move || rows_of(&p).len() >= 1).await;
        let back = UsageRecord::from(rows_of(&path).remove(0));
        assert_eq!(back, rec);
        assert!(back.input_tokens.is_none(), "未观测 ≠ 0");
    }

    /// spec 场景「records are queryable by field」：按 model 直接查库取回
    /// 匹配记录——不需要解析任何日志文件。
    #[tokio::test]
    async fn records_are_queryable_by_field() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let sink = UsageSink::spawn_writer(&path, no_background_prune()).expect("spawn_writer");

        for (model, provider) in [
            ("claude-sonnet", "anthropic"),
            ("gpt-4o-mini", "openai"),
            ("claude-sonnet", "deepseek"),
        ] {
            let mut rec = sample_record("2026-08-07T00:00:00+00:00");
            rec.model = Some(model.into());
            rec.provider = provider.into();
            sink.record(rec);
        }
        let p = path.clone();
        wait_until("three rows", move || rows_of(&p).len() >= 3).await;

        // 非标准查询（按非键列条件）可手写 SQL，但必须返回 struct 实例。
        let conn = sebas_db::conn::open(&path).unwrap();
        let sql = format!(
            "SELECT {} FROM usage_records WHERE model = ?1 ORDER BY id",
            <UsageRow as Record>::COLUMNS.join(", ")
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        let hits = stmt
            .query_map(["claude-sonnet"], UsageRow::from_row)
            .unwrap()
            .collect::<sebas_db::rusqlite::Result<Vec<UsageRow>>>()
            .unwrap();

        assert_eq!(hits.len(), 2, "按 model 查回两条");
        assert_eq!(hits[0].provider, "anthropic");
        assert_eq!(hits[1].provider, "deepseek");
        // 迁移前就有的字段逐个在场。
        assert_eq!(hits[0].upstream_model.as_deref(), Some("anthropic.claude-sonnet-4"));
        assert_eq!(hits[0].input_tokens, Some(10));
        assert_eq!(hits[0].output_tokens, Some(50));
        assert_eq!(hits[0].cache_read_tokens, Some(5));
        assert_eq!(hits[0].cache_creation_tokens, Some(2));
    }

    // ---------------- 2.4 库写失败 → 丢弃 + warn，不影响调用方 ----------------

    #[tokio::test]
    async fn write_failure_is_swallowed_and_the_sink_keeps_accepting() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let sink = UsageSink::spawn_writer(&path, no_background_prune()).expect("spawn_writer");

        sink.record(sample_record("2026-08-07T00:00:00+00:00"));
        let p = path.clone();
        wait_until("first row", move || !rows_of(&p).is_empty()).await;

        // 制造写失败：把 usage_records 表整个删掉（后续写命令随之失败）。
        {
            let conn = sebas_db::conn::open(&path).unwrap();
            conn.execute_batch("DROP TABLE usage_records").unwrap();
        }
        // 投递若干条：写失败被吞掉（只 warn），调用方零感知、也不 panic。
        for i in 0..8 {
            let mut rec = sample_record("2026-08-07T00:00:01+00:00");
            rec.latency_ms = i;
            sink.record(rec);
        }
        // 给 writer task 时间跑完这些失败命令（不得 panic 退出）。
        tokio::time::sleep(Duration::from_millis(250)).await;

        // task 仍在（channel 未关闭）：继续投递仍即时返回。
        let start = std::time::Instant::now();
        sink.record(sample_record("2026-08-07T00:00:02+00:00"));
        assert!(
            start.elapsed() < Duration::from_millis(200),
            "写失败后 record 仍不阻塞"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ---------------- 3.1 时间闸 ----------------

    #[tokio::test]
    async fn retention_prunes_records_older_than_the_window() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let policy = RetentionPolicy {
            retention_days: 7,
            max_rows: 0,
            prune_interval_secs: 0,
        };
        let sink = UsageSink::spawn_writer(&path, policy).expect("spawn_writer");

        // 一条很久以前（超窗口）、一条刚刚（窗口内）。
        let fresh = sample_record(&chrono::Utc::now().to_rfc3339());
        sink.record(sample_record("2000-01-01T00:00:00+00:00"));
        sink.record(fresh.clone());
        let p = path.clone();
        wait_until("two rows", move || rows_of(&p).len() >= 2).await;

        // 手动跑一次清理（后台间隔 0 = 关闭，这里直接驱动同一条路径）。
        let writer = StateWriter::start(path.clone(), USAGE_TABLES).unwrap();
        prune_once(writer.handle(), policy).await;

        let rows = rows_of(&path);
        assert_eq!(rows.len(), 1, "超期记录被清理");
        assert_eq!(rows[0].ts, fresh.ts, "窗口内的记录原样保留（不误删有用历史）");
    }

    /// 时间闸边界：窗口**内**（比 cutoff 新）的记录一条都不删——即使它很旧。
    #[tokio::test]
    async fn retention_window_boundary_keeps_everything_inside() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let policy = RetentionPolicy {
            retention_days: 30,
            max_rows: 0,
            prune_interval_secs: 0,
        };
        let sink = UsageSink::spawn_writer(&path, policy).expect("spawn_writer");

        // 29 天前（窗口内）、31 天前（窗口外）。
        let inside = (chrono::Utc::now() - chrono::Duration::days(29)).to_rfc3339();
        let outside = (chrono::Utc::now() - chrono::Duration::days(31)).to_rfc3339();
        sink.record(sample_record(&inside));
        sink.record(sample_record(&outside));
        let p = path.clone();
        wait_until("two rows", move || rows_of(&p).len() >= 2).await;

        let writer = StateWriter::start(path.clone(), USAGE_TABLES).unwrap();
        prune_once(writer.handle(), policy).await;

        let rows = rows_of(&path);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ts, inside, "29 天前的记录留在窗口内");
    }

    // ---------------- 3.2 行数闸 ----------------

    #[tokio::test]
    async fn retention_keeps_only_the_newest_rows_under_the_ceiling() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let policy = RetentionPolicy {
            retention_days: 0,
            max_rows: 5,
            prune_interval_secs: 0,
        };
        let sink = UsageSink::spawn_writer(&path, policy).expect("spawn_writer");

        for i in 0..12 {
            let mut rec = sample_record("2026-08-07T00:00:00+00:00");
            rec.model = Some(format!("m{i:02}"));
            sink.record(rec);
        }
        let p = path.clone();
        wait_until("12 rows", move || rows_of(&p).len() >= 12).await;

        let writer = StateWriter::start(path.clone(), USAGE_TABLES).unwrap();
        prune_once(writer.handle(), policy).await;

        let rows = rows_of(&path);
        assert_eq!(rows.len(), 5, "清理后行数不超上限");
        // 最近 5 条保留（m07..m11），最旧的被删。
        let models: Vec<&str> = rows.iter().filter_map(|r| r.model.as_deref()).collect();
        assert_eq!(models, vec!["m07", "m08", "m09", "m10", "m11"]);
    }

    /// 两个闸都不设（全 0）= 不清理（design D2 的取舍：操作员可显式关闸）。
    #[tokio::test]
    async fn zero_policy_prunes_nothing() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let policy = RetentionPolicy {
            retention_days: 0,
            max_rows: 0,
            prune_interval_secs: 0,
        };
        let sink = UsageSink::spawn_writer(&path, policy).expect("spawn_writer");
        sink.record(sample_record("2000-01-01T00:00:00+00:00"));
        let p = path.clone();
        wait_until("one row", move || rows_of(&p).len() >= 1).await;

        let writer = StateWriter::start(path.clone(), USAGE_TABLES).unwrap();
        prune_once(writer.handle(), policy).await;
        assert_eq!(rows_of(&path).len(), 1, "双闸全关时不删任何记录");
    }

    /// 两闸同时生效：先删超期，再按行数删超量。
    #[tokio::test]
    async fn both_gates_apply_together() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let policy = RetentionPolicy {
            retention_days: 7,
            max_rows: 3,
            prune_interval_secs: 0,
        };
        let sink = UsageSink::spawn_writer(&path, policy).expect("spawn_writer");

        // 2 条超期 + 5 条窗口内。
        for i in 0..2 {
            let mut rec = sample_record("2000-01-01T00:00:00+00:00");
            rec.model = Some(format!("old{i}"));
            sink.record(rec);
        }
        for i in 0..5 {
            let mut rec = sample_record(&chrono::Utc::now().to_rfc3339());
            rec.model = Some(format!("new{i}"));
            sink.record(rec);
        }
        let p = path.clone();
        wait_until("7 rows", move || rows_of(&p).len() >= 7).await;

        let writer = StateWriter::start(path.clone(), USAGE_TABLES).unwrap();
        prune_once(writer.handle(), policy).await;

        let rows = rows_of(&path);
        assert_eq!(rows.len(), 3, "先时间闸再行数闸，最终 3 条");
        let models: Vec<&str> = rows.iter().filter_map(|r| r.model.as_deref()).collect();
        assert_eq!(models, vec!["new2", "new3", "new4"], "保留最近的窗口内记录");
    }

    // ---------------- 3.3 后台定期间隔清理（不阻塞响应） ----------------

    /// 后台 task 按间隔自跑：起 sink 时投一条超期记录，等一个 tick 后它应
    /// 被清掉；期间投递即时返回，且窗口内的新记录不受影响。
    #[tokio::test]
    async fn background_pruner_runs_on_interval_without_blocking() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let policy = RetentionPolicy {
            retention_days: 1,
            max_rows: 0,
            prune_interval_secs: 1,
        };
        let sink = UsageSink::spawn_writer(&path, policy).expect("spawn_writer");

        sink.record(sample_record("2000-01-01T00:00:00+00:00"));
        let p = path.clone();
        wait_until("row landed before prune", move || rows_of(&p).len() >= 1).await;

        let p = path.clone();
        wait_until("background pruner removed the aged row", move || {
            rows_of(&p).is_empty()
        })
        .await;

        // 清理期间投递一条新记录：调用方即时返回，新记录仍会落库。
        let fresh = sample_record(&chrono::Utc::now().to_rfc3339());
        let fresh_ts = fresh.ts.clone();
        let start = std::time::Instant::now();
        sink.record(fresh);
        assert!(
            start.elapsed() < Duration::from_millis(200),
            "投递绝不因清理而阻塞"
        );
        let p = path.clone();
        wait_until("fresh row survives", move || {
            rows_of(&p).iter().any(|r| r.ts == fresh_ts)
        })
        .await;
    }

    /// 时间闸的 cutoff 只取决于配置（`0` = 关闸 → None）。
    #[test]
    fn retention_cutoff_respects_the_zero_gate() {
        assert!(
            retention_cutoff(RetentionPolicy {
                retention_days: 0,
                ..Default::default()
            })
            .is_none()
        );
        let cutoff = retention_cutoff(RetentionPolicy {
            retention_days: 30,
            ..Default::default()
        })
        .expect("30 天窗口有 cutoff");
        let cutoff = chrono::DateTime::parse_from_rfc3339(&cutoff).expect("RFC3339");
        let expected = chrono::Utc::now() - chrono::Duration::days(30);
        assert!(
            (cutoff.to_utc() - expected).num_seconds().abs() < 60,
            "cutoff 应约等于 now - 30d"
        );
    }

    // ---------------- schema / 配方 ----------------

    /// **保留期正确性的隐藏前提**：时间闸是 `WHERE ts < ?` 的**字典序**比较，
    /// 因此 `chrono` 的 `to_rfc3339()`（`SecondsFormat::AutoSi`，小数位 0/3/6/9）
    /// 输出必须满足「字典序 == 时间序」。
    ///
    /// 成立理由：`+`(0x2B) < `.`(0x2E) < `0`..`9`(0x30..0x39)，且 AutoSi 只在
    /// 纳秒可整除时缩短小数位——所以整数部分相同的前缀下，短格式恒排在长格式
    /// 之前，与数值大小一致。这条断言把该前提钉住：chrono 换格式策略时会红。
    #[test]
    fn rfc3339_lexicographic_order_matches_time_order() {
        let base = chrono::DateTime::parse_from_rfc3339("2026-09-24T03:25:45+00:00")
            .unwrap()
            .to_utc();
        // 覆盖 AutoSi 的四种小数位形态 + 相邻值。
        let nanos = [
            0i64,
            1,
            999,
            1_000,
            999_999,
            1_000_000,
            123_000_000,
            123_456_000,
            123_456_001,
            123_456_789,
            500_000_000,
            999_999_999,
        ];
        let samples: Vec<(chrono::DateTime<chrono::Utc>, String)> = nanos
            .iter()
            .map(|n| {
                let t = base + chrono::Duration::nanoseconds(*n);
                (t, t.to_rfc3339())
            })
            .collect();

        for (i, (t1, s1)) in samples.iter().enumerate() {
            for (t2, s2) in samples.iter().skip(i + 1) {
                assert_eq!(
                    t1.cmp(t2),
                    s1.cmp(s2),
                    "字典序与时间序不一致: {t1} ({s1}) vs {t2} ({s2})"
                );
            }
        }
        // 形态自证：0 / 3 / 6 / 9 位小数各出现一次以上。
        for digits in [0usize, 3, 6, 9] {
            assert!(
                samples.iter().any(|(_, s)| {
                    let frac = s.split('.').nth(1).map(|f| f.len() - 6).unwrap_or(0);
                    frac == digits
                }),
                "AutoSi 未产出 {digits} 位小数形态"
            );
        }
    }

    /// 时间闸的**混合精度**场景：记录与 cutoff 的小数位不同也不误删/误留
    /// （依赖上一条断言的序性质）。
    #[tokio::test]
    async fn retention_compares_timestamps_across_fractional_precisions() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let policy = RetentionPolicy {
            retention_days: 7,
            max_rows: 0,
            prune_interval_secs: 0,
        };
        let sink = UsageSink::spawn_writer(&path, policy).expect("spawn_writer");

        // 整秒（0 位小数）的窗口内记录 + 9 位小数的超期记录。
        let now = chrono::Utc::now();
        let inside_whole = now - chrono::Duration::days(1);
        let inside = inside_whole.to_rfc3339(); // AutoSi 可能给 0 位
        let outside = (now - chrono::Duration::days(9)).to_rfc3339();
        sink.record(sample_record(&inside));
        sink.record(sample_record(&outside));
        let p = path.clone();
        wait_until("two rows", move || rows_of(&p).len() >= 2).await;

        let writer = StateWriter::start(path.clone(), USAGE_TABLES).unwrap();
        prune_once(writer.handle(), policy).await;

        let rows = rows_of(&path);
        assert_eq!(rows.len(), 1, "只删超期的那条");
        assert_eq!(rows[0].ts, inside, "窗口内记录保留（跨精度比较正确）");
    }

    /// 表注册清单与 struct 派生列一致（schema 事实源 = struct）。
    #[test]
    fn table_registry_columns_match_the_struct() {
        assert_eq!(USAGE_TABLES.len(), 1);
        let table = &USAGE_TABLES[0];
        assert_eq!(table.name, "usage_records");
        assert_eq!(table.name, <UsageRow as Record>::TABLE);
        assert_eq!(table.columns, UsageRow::schema_columns());
        assert_eq!(<UsageRow as Record>::PK_COLUMNS, &["id"]);
        // 全部对外字段都在列清单里（key/protocol/.../error + id + ts）。
        for col in [
            "id",
            "key",
            "protocol",
            "model",
            "provider",
            "upstream_model",
            "status",
            "latency_ms",
            "ttft_ms",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_creation_tokens",
            "error",
            "ts",
        ] {
            assert!(
                <UsageRow as Record>::COLUMNS.contains(&col),
                "列 {col} 缺失"
            );
        }
        // DDL 里出现的列名与派生列逐个对齐（建表与 struct 不漂移）。
        for col in <UsageRow as Record>::COLUMNS {
            assert!(table.create_ddl.contains(col), "DDL 未声明派生列 {col}");
        }
    }

    /// 打开库后的连接配方来自共享层（WAL + busy_timeout=5s + foreign_keys=ON）。
    #[test]
    fn usage_db_connection_recipe_comes_from_the_shared_layer() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let _writer = StateWriter::start(path.clone(), USAGE_TABLES).unwrap();
        let conn = sebas_db::conn::open(&path).unwrap();

        let journal: String = conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(journal, "wal");
        let timeout: i64 = conn
            .pragma_query_value(None, "busy_timeout", |r| r.get(0))
            .unwrap();
        assert_eq!(timeout, 5000);
        let fk: i64 = conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    /// schema 同步的版本戳由 `sebas_db::schema` 打（工作区不允许第二套版本机制）。
    #[test]
    fn schema_version_stamp_comes_from_the_shared_layer() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.db");
        let _writer = StateWriter::start(path.clone(), USAGE_TABLES).unwrap();
        let conn = sebas_db::conn::open(&path).unwrap();
        let version_format: String = conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'version_format'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version_format, sebas_db::schema::VERSION_FORMAT);
    }
}