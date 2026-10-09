//! 本地回合用量账本（add-local-usage-statistics，能力 `local-usage-capture`）。
//!
//! 不经 router 的 agent 回合（ACP 全部 + native 直连）在 core 独占写入的
//! `usage_local.db` 落账——与 router 的 `usage.db` 平行同构，**双算规避在
//! 写入侧**（design D1）：经 router 的回合 router 已记，core 不记；一条请求
//! 恰好被一个源计数。
//!
//! # 存储形态（design D2）
//!
//! 落点由 sebas home 映射表派生（`StatePath::UsageLocalDb` →
//! `<SEBAS_HOME>/usage_local.db`，`SEBAS_USAGE_LOCAL_DB` 显式覆盖）。连接
//! 配方、schema 同步与单写执行模型全部取自共享持久层 `sebas-db`——本模块
//! 不自建连接管理、不手抄 pragma。表注册 DDL 在 [`crate::sebas_state::repo`]
//! （工作区唯一手写 DDL 处）；行 struct [`LocalUsageRow`] 归本消费模块
//! （归属按写入者——core 是唯一写入者，router 绝不打开这个文件）。
//!
//! # 行映射
//!
//! `local_usage_records` 表一行对应一个回合（逐回合一行，design D5）。行
//! 携带共享记录形状 [`sebas_domain::usage::UsageRecord`] 的**每一个字段**
//! （可空 token 计数原样保 `NULL`，不以零冒充）；`protocol` 承载执行体标签
//! （`acp` / `native`），`status` 承载回合终态（200 = 完成、499 = 取消、
//! 500 = 失败）。
//!
//! # 异步 sink 语义（与 router sink 逐条同款）
//!
//! 容量 256 的有界通道、满则丢弃 + warn、**绝不阻塞或失败在途回合**、写
//! 失败不影响会话——usage 是统计旁路。
//!
//! # 保留期（design D2）
//!
//! 与 router 侧同款双闸（默认 30 天 / 20 万行 / 每小时，`[usage_local]`
//! 键族独立可配），后台定期间隔清理，清理计数写日志。
//!
//! # 查询（design D4）
//!
//! [`query_timeseries`] 走与 router 同一份域层聚合纯函数（同输入同桶形）；
//! [`usage_timeseries_outcome`] 是 `source=router|local|all` 三口径的 core
//! 侧编排单点（通道服务端与内嵌 webui 后端共用）。

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use sebas_db::record::Record;
use sebas_db::schema::TableSchema;
use sebas_db::writer::{StateHandle, StateWriter};
use sebas_domain::state_paths::StatePath;
use sebas_domain::usage::{
    merge_timeseries, timeseries, Timeseries, TimeseriesParams, UsageRecord,
};
use sebas_schema_derive::{ActiveRecord, SchemaColumns};
use tokio::sync::mpsc;

// ---- 落账门控（design D1：双算规避的写入侧规则）----

/// native 回合是否落**本地**账：内核装配时注入了 `SEBAS_AGENT_ROUTER_URL`
/// 即经 router（router 已记）→ 不本地记；直连 provider → 本地记。ACP 会话
/// 的 agent 永不经过 router，恒落本地（不走本判定）。
pub fn native_records_locally(router_url: Option<&str>) -> bool {
    match router_url {
        None => true,
        Some(u) => u.trim().is_empty(),
    }
}

/// 进程装配态的判定（读 env；内核经 [`crate::agent_backend`] 装配时注入）。
pub fn native_records_locally_from_env() -> bool {
    native_records_locally(std::env::var("SEBAS_AGENT_ROUTER_URL").ok().as_deref())
}

/// 回合终态 → 记录的 `status`（domain 常量原位再导出：本 crate 既有引用面）。
pub use sebas_domain::usage::{
    PROTOCOL_ACP, PROTOCOL_NATIVE, TURN_STATUS_CANCELLED, TURN_STATUS_FAILED,
    TURN_STATUS_FINISHED,
};

// ---- 行 struct（写入者 = core，归属本模块）----

/// `local_usage_records` 表的一行（一表一 struct，标准 CRUD 由 derive 生成）。
///
/// `id` 是自增主键：行按**完成顺序**追加，`id` 升序即完成顺序。`ts` 是
/// RFC3339 UTC 字符串——保留期时间闸按**字典序**比较即可（同格式同时区的
/// RFC3339 串，字典序 = 时间序）。列集与共享记录形状逐字段对应（含
/// `key`/`protocol`/`upstream_model`/`ttft_ms` 保留位——本地行携带共享形状
/// 的每一个字段，NULL 保 NULL）。
#[derive(Debug, Clone, PartialEq, Eq, SchemaColumns, ActiveRecord)]
#[active_record(table = "local_usage_records")]
#[active_record(pk = "id")]
pub struct LocalUsageRow {
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

impl From<&UsageRecord> for LocalUsageRow {
    fn from(r: &UsageRecord) -> Self {
        LocalUsageRow {
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

impl From<LocalUsageRow> for UsageRecord {
    fn from(r: LocalUsageRow) -> Self {
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

/// 后台 writer task 的容量（与 router sink 同值）。满则 `record` 用
/// `try_send` 丢弃并 warn。
const CHANNEL_CAPACITY: usize = 256;

/// 一次批量提交最多聚合的记录数（与 router sink 同语义：批量降低提交次数，
/// 已入通道的记录一定被提交）。
const BATCH_MAX: usize = 64;

/// 保留期配置（三个闸；`0` 分别表示关闭）。形状与默认值与 router 侧同款
/// （复用 router 的 [`sebas_router::usage::RetentionPolicy`]：30 天 / 20 万行
/// / 每小时）——**配置入口独立**（`[usage_local]` 键族），两个账本互不引用
/// 对方的键。
pub use sebas_router::usage::RetentionPolicy as LocalRetentionPolicy;

/// 本地用量库写入器句柄。`Clone` 因 `mpsc::Sender` 可克隆（装配点需 Clone）。
/// drop 不等 sink 关闭——后台 task 在 `recv → None` 时自然退出。
#[derive(Clone)]
pub struct LocalUsageSink {
    tx: mpsc::Sender<UsageRecord>,
    /// 只读查询句柄：与写入/清理共用同一条单写线程命令队列，聚合 SELECT
    /// 在其上串行执行（毫秒级，不影响会话路径）。
    query: StateHandle,
}

impl LocalUsageSink {
    /// 本地用量库的查询句柄（`source=local|all` 聚合 SELECT 用）。
    pub fn query_handle(&self) -> &StateHandle {
        &self.query
    }

    /// 打开本地用量库并起后台 writer task。路径来自
    /// `StatePath::UsageLocalDb`（缺省）或调用方显式传入（测试）。
    ///
    /// 失败 → io::Error 转嫁调用方（run.rs 映射为启动失败拒绝——与 router
    /// 侧 build_state 的既有语义对齐：账本开不出来不静默降级）。
    ///
    /// `tokio::spawn` 要求调用线程处于 tokio 运行时上下文（run.rs 在 async
    /// 装配内调用；测试用 `#[tokio::test]`）。
    pub fn spawn(
        path: impl AsRef<Path>,
        policy: LocalRetentionPolicy,
    ) -> io::Result<Self> {
        let path: PathBuf = path.as_ref().to_path_buf();
        // 父目录先建：避免 writer task 启动时 open 失败导致所有记录丢弃。
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && parent != Path::new(".")
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("创建本地用量库父目录 {parent:?} 失败（库 {path:?}）: {e}"),
                )
            })?;
        }
        // 单写 actor（sebas-db）：专用线程 + 命令串行；启动即完成 schema 同步。
        let writer = StateWriter::start(path.clone(), local_usage_tables())
            .map_err(|e| io::Error::other(format!("本地用量库 {path:?} 初始化失败: {e}")))?;
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
                let rows: Vec<LocalUsageRow> = batch.iter().map(LocalUsageRow::from).collect();
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
                    // 库不可用/写失败 → 丢弃 + warn，绝不影响会话（sink 语义：
                    // usage 是统计旁路，绝不阻断在途回合）。
                    tracing::warn!(
                        error = %e,
                        dropped = n,
                        "local usage db write failed; dropping records"
                    );
                }
            }
        });

        // 后台保留期清理：独立 task，间隔触发，绝不阻塞会话（清理在 writer
        // 线程串行执行，与写入共用同一条命令队列）。首个 tick 在一个间隔之后
        // （与 router 侧同款：启动瞬间不删存量）。
        if policy.prune_interval_secs > 0 && (policy.retention_days > 0 || policy.max_rows > 0) {
            let prune_handle = handle.clone();
            let interval = Duration::from_secs(policy.prune_interval_secs);
            tokio::spawn(async move {
                let start = tokio::time::Instant::now() + interval;
                let mut ticker = tokio::time::interval_at(start, interval);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    ticker.tick().await;
                    prune_once(&prune_handle, policy).await;
                }
            });
        }

        Ok(LocalUsageSink { tx, query: handle })
    }

    /// 投递一条记录。mpsc(256) 满则 warn 丢弃（warn 串恒为静态字面量，不含
    /// 记录材料）。channel 关闭（writer task 已退出）同样 warn 丢弃。**绝不
    /// 阻塞调用方**——落账钩子在回合结算路径上，满载只丢统计不丢回合。
    pub fn record(&self, rec: UsageRecord) {
        if let Err(e) = self.tx.try_send(rec) {
            match e {
                mpsc::error::TrySendError::Full(_) => {
                    tracing::warn!(
                        "local usage sink channel full (cap {CHANNEL_CAPACITY}); dropping record"
                    );
                }
                mpsc::error::TrySendError::Closed(_) => {
                    tracing::warn!("local usage sink writer closed; dropping record");
                }
            }
        }
    }
}

/// 表注册清单（引用 `sebas_state` 注册表的 DDL——工作区唯一手写 DDL 处）。
pub fn local_usage_tables() -> &'static [TableSchema] {
    crate::sebas_state::repo::USAGE_LOCAL_TABLES
}

/// 跑一次保留期清理（两个闸，与 router 侧 `prune_once` 同款语义）。清理
/// 计数写 INFO 日志——「数据被删」必须可见。任何错误只 warn，不向上传播。
pub async fn prune_once(handle: &StateHandle, policy: LocalRetentionPolicy) {
    let cutoff = retention_cutoff(policy);
    let max_rows = policy.max_rows;
    let result = handle
        .exec(move |conn| -> Result<(usize, usize), String> {
            let mut aged = 0usize;
            if let Some(cutoff) = cutoff.as_deref() {
                aged = conn
                    .execute("DELETE FROM local_usage_records WHERE ts < ?1", [cutoff])
                    .map_err(|e| e.to_string())?;
            }
            let mut excess = 0usize;
            if max_rows > 0 {
                // 行数闸：删掉最旧的「超出行数」条——按 id 升序（= 完成顺序）
                // 保留最近 max_rows 条。
                excess = conn
                    .execute(
                        "DELETE FROM local_usage_records WHERE id NOT IN (
                             SELECT id FROM local_usage_records ORDER BY id DESC LIMIT ?1
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
                    "local usage retention pruned records"
                );
            }
        }
        Err(e) => tracing::warn!(error = %e, "local usage retention prune failed"),
    }
}

/// 时间闸的 cutoff（RFC3339 UTC 字符串，字典序 = 时间序）。`None` = 关闸。
fn retention_cutoff(policy: LocalRetentionPolicy) -> Option<String> {
    if policy.retention_days == 0 {
        return None;
    }
    let days = i64::try_from(policy.retention_days).unwrap_or(i64::MAX);
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
    Some(cutoff.to_rfc3339())
}

// ---- 查询面（design D4：source 三口径的 core 侧编排单点）----

/// core 侧 usage 时序查询的失败面（`source=router` 专用：local/all 恒 200）。
#[derive(Debug, Clone, PartialEq)]
pub enum UsageQueryOutcome {
    /// 200：聚合载荷（local/all 为 core 合成；router 为原样透传）。
    Ok(Value),
    /// router 应答了非 200：状态码与 JSON 体原样透传（参数 400 等）。
    RouterError { status: u16, body: Value },
    /// router 不可达（未启用 / 拒绝 / 超时）——仅 `source=router` 走这条：
    /// cause 以 `router_unreachable` 前缀点名（webui 呈「router 不可达」）。
    RouterUnreachable { cause: String },
}

/// source 词表（`/api/usage/timeseries?source=`）。唯一定义在中立域层
/// （sebas-webui 路由层与 core 取数端共用同一合法域），这里原位再导出。
pub use sebas_domain::usage::usage_source;

/// 本地库聚合（纯本库；router 不参与）。`handle` 为 `None` = 账本未装配
/// （仅测试/降级装配）：如实返回全零窗口（结构在场，数据诚实为零）。
pub async fn local_timeseries(
    handle: Option<&StateHandle>,
    params: TimeseriesParams,
    now_utc: chrono::DateTime<chrono::Utc>,
) -> Result<Timeseries, String> {
    let Some(handle) = handle else {
        return Ok(timeseries(&[], params, now_utc));
    };
    let start = params.window_start_utc(now_utc).to_rfc3339();
    let rows = handle
        .exec(move |conn| -> Result<Vec<LocalUsageRow>, String> {
            let sql = format!(
                "SELECT {} FROM local_usage_records WHERE ts >= ?1",
                <LocalUsageRow as Record>::COLUMNS.join(", ")
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([&start], LocalUsageRow::from_row)
                .map_err(|e| e.to_string())?
                .collect::<sebas_db::rusqlite::Result<Vec<LocalUsageRow>>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        })
        .await?;
    let records: Vec<UsageRecord> = rows.into_iter().map(UsageRecord::from).collect();
    Ok(timeseries(&records, params, now_utc))
}

/// `source` 三口径的编排（design D4；通道服务端与内嵌 webui 后端共用）：
///
/// - `router`：既有反代语义——成功原样透传；不可达 →
///   [`UsageQueryOutcome::RouterUnreachable`]（调用方映射结构化 cause）；
/// - `local`：纯本地聚合，恒 [`UsageQueryOutcome::Ok`]；
/// - `all`：本地聚合 + 尽力反代合并——router 不可达**仍 200**，响应带
///   `router_cause` 如实标注缺席源（写入侧单源规则保证合并不重算）。
pub async fn usage_timeseries_outcome(
    local: Option<&StateHandle>,
    router_listen: Option<&str>,
    source: &str,
    params: TimeseriesParams,
    now_utc: chrono::DateTime<chrono::Utc>,
) -> UsageQueryOutcome {
    match source {
        usage_source::ROUTER => {
            let Some(listen) = router_listen else {
                return UsageQueryOutcome::RouterUnreachable {
                    cause: crate::router_admin::ROUTER_NOT_CONFIGURED_CAUSE.to_string(),
                };
            };
            match crate::router_admin::fetch_usage_timeseries(
                listen,
                params.granularity.as_str(),
                params.days,
                params.tz_offset_min,
            )
            .await
            {
                Ok(payload) => UsageQueryOutcome::Ok(payload),
                Err(crate::router_admin::UsageProxyError::RouterError { status, body }) => {
                    UsageQueryOutcome::RouterError { status, body }
                }
                Err(e @ crate::router_admin::UsageProxyError::Unreachable { .. }) => {
                    UsageQueryOutcome::RouterUnreachable { cause: e.cause() }
                }
            }
        }
        usage_source::LOCAL => match local_timeseries(local, params, now_utc).await {
            Ok(ts) => UsageQueryOutcome::Ok(serde_json::to_value(&ts).unwrap_or(Value::Null)),
            Err(e) => UsageQueryOutcome::RouterUnreachable {
                cause: format!("local_usage_db_error: {e}"),
            },
        },
        _ => {
            // all：本地聚合 + 尽力合并。router 缺席/失败都不是错误——
            // 本地数据照常返回，`router_cause` 标注缺席源。
            let local_ts = match local_timeseries(local, params, now_utc).await {
                Ok(ts) => ts,
                Err(e) => {
                    return UsageQueryOutcome::RouterUnreachable {
                        cause: format!("local_usage_db_error: {e}"),
                    };
                }
            };
            let (router_ts, cause) = match router_listen {
                None => (None, Some(crate::router_admin::ROUTER_NOT_CONFIGURED_CAUSE.to_string())),
                Some(listen) => {
                    match crate::router_admin::fetch_usage_timeseries(
                        listen,
                        params.granularity.as_str(),
                        params.days,
                        params.tz_offset_min,
                    )
                    .await
                    {
                        Ok(payload) => {
                            // router 载荷反序列化（新增字段缺省兼容）；
                            // 解析失败按「缺席源」如实标注，不冒充零。
                            match serde_json::from_value::<Timeseries>(payload) {
                                Ok(ts) => (Some(ts), None),
                                Err(e) => (
                                    None,
                                    Some(format!(
                                        "{}: router 聚合应答不可解析: {e}",
                                        crate::router_admin::ROUTER_UNREACHABLE_CAUSE
                                    )),
                                ),
                            }
                        }
                        Err(e) => (None, Some(e.cause())),
                    }
                }
            };
            let merged = match router_ts {
                Some(rt) => merge_timeseries(&local_ts, &rt),
                None => {
                    let mut m = local_ts;
                    m.router_cause = cause;
                    m
                }
            };
            UsageQueryOutcome::Ok(serde_json::to_value(&merged).unwrap_or(Value::Null))
        }
    }
}

/// 缺省落点（sebas home 映射表派生，env 可覆盖）。
pub fn default_db_path() -> PathBuf {
    StatePath::UsageLocalDb.resolve()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_record(ts: &str) -> UsageRecord {
        UsageRecord {
            ts: ts.into(),
            key: String::new(),
            protocol: PROTOCOL_ACP.to_string(),
            model: Some("claude-sonnet".into()),
            provider: String::new(),
            upstream_model: None,
            status: TURN_STATUS_FINISHED,
            latency_ms: 123,
            ttft_ms: None,
            input_tokens: Some(10),
            output_tokens: Some(50),
            cache_read_tokens: Some(5),
            cache_creation_tokens: Some(2),
            error: None,
        }
    }

    /// 直开库读全表（按 id = 完成顺序）。
    fn rows_of(path: &Path) -> Vec<LocalUsageRow> {
        let conn = sebas_db::conn::open(path).expect("open local usage db");
        let mut stmt = conn
            .prepare(
                "SELECT id, key, protocol, model, provider, upstream_model, status,
                        latency_ms, ttft_ms, input_tokens, output_tokens,
                        cache_read_tokens, cache_creation_tokens, error, ts
                 FROM local_usage_records ORDER BY id",
            )
            .expect("prepare");
        stmt.query_map([], LocalUsageRow::from_row)
            .expect("query")
            .collect::<sebas_db::rusqlite::Result<Vec<_>>>()
            .expect("collect")
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

    fn no_background_prune() -> LocalRetentionPolicy {
        LocalRetentionPolicy {
            prune_interval_secs: 0,
            ..Default::default()
        }
    }

    // ---- 2.1：行 struct / 表注册 / 落点派生 ----

    #[test]
    fn registry_columns_match_the_struct_and_ddl() {
        let tables = local_usage_tables();
        assert_eq!(tables.len(), 1);
        let table = &tables[0];
        assert_eq!(table.name, "local_usage_records");
        assert_eq!(table.name, <LocalUsageRow as Record>::TABLE);
        assert_eq!(table.columns, LocalUsageRow::schema_columns());
        assert_eq!(<LocalUsageRow as Record>::PK_COLUMNS, &["id"]);
        // 共享记录形状的每个字段都在列清单里（本地行携带全形状）。
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
                <LocalUsageRow as Record>::COLUMNS.contains(&col),
                "列 {col} 缺失"
            );
        }
        // DDL 里出现的列名与派生列逐个对齐（建表与 struct 不漂移）。
        let registered: String = std::iter::once(table.create_table_ddl)
            .chain(table.index_ddls.iter().copied())
            .collect::<Vec<_>>()
            .join("\n");
        for col in <LocalUsageRow as Record>::COLUMNS {
            assert!(registered.contains(col), "DDL 未声明派生列 {col}");
        }
    }

    #[test]
    fn default_db_path_derives_from_sebas_home_mapping() {
        // 不动进程 env 的口径：映射表行的相对路径与覆盖变量名。
        assert_eq!(StatePath::UsageLocalDb.rel_path(), "usage_local.db");
        assert_eq!(
            StatePath::UsageLocalDb.override_var(),
            Some("SEBAS_USAGE_LOCAL_DB")
        );
        let p = default_db_path();
        assert!(
            p.ends_with("usage_local.db") || p.file_name().is_some(),
            "缺省落点 = 映射表解析: {p:?}"
        );
    }

    #[test]
    fn row_round_trips_every_field_of_the_shared_shape() {
        let rec = sample_record("2026-10-10T00:00:00+00:00");
        let row = LocalUsageRow::from(&rec);
        let back = UsageRecord::from(row);
        assert_eq!(back, rec, "行往返逐字段保真");
        // NULL 保 NULL（不以零冒充——spec 场景「null-able token counts
        // preserved as null, not zero」）。
        let empty = UsageRecord {
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            error: Some("boom".into()),
            status: TURN_STATUS_FAILED,
            ..sample_record("2026-10-10T00:00:01+00:00")
        };
        let back = UsageRecord::from(LocalUsageRow::from(&empty));
        assert_eq!(back, empty);
        assert!(back.input_tokens.is_none());
    }

    // ---- 2.2：sink 语义与保留期 ----

    #[tokio::test]
    async fn records_land_in_db_in_completion_order() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage_local.db");
        let sink = LocalUsageSink::spawn(&path, no_background_prune()).expect("spawn");

        let rec1 = sample_record("2026-08-07T00:00:00+00:00");
        let mut rec2 = rec1.clone();
        rec2.model = Some("second".into());
        rec2.protocol = PROTOCOL_NATIVE.to_string();
        sink.record(rec1);
        sink.record(rec2);

        let p = path.clone();
        wait_until("two rows committed", move || rows_of(&p).len() >= 2).await;

        let rows = rows_of(&path);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].model.as_deref(), Some("claude-sonnet"));
        assert_eq!(rows[1].model.as_deref(), Some("second"));
        assert_eq!(rows[1].protocol, "native");
        assert!(rows[0].id.unwrap() < rows[1].id.unwrap(), "id 严格递增");
    }

    /// sink 语义：通道满时 `record` 立即返回（绝不阻塞回合结算路径）。
    #[tokio::test]
    async fn record_never_blocks_even_when_the_channel_overflows() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage_local.db");
        let sink = LocalUsageSink::spawn(&path, no_background_prune()).expect("spawn");

        let start = std::time::Instant::now();
        for i in 0..(CHANNEL_CAPACITY * 4) {
            let mut rec = sample_record("2026-08-07T00:00:00+00:00");
            rec.model = Some(format!("m{i}"));
            sink.record(rec);
        }
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "record 绝不阻塞调用方"
        );

        let p = path.clone();
        wait_until("some rows committed", move || !rows_of(&p).is_empty()).await;
    }

    /// 写失败被吞掉（只 warn），调用方零感知、sink 继续接受。
    #[tokio::test]
    async fn write_failure_is_swallowed_and_the_sink_keeps_accepting() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage_local.db");
        let sink = LocalUsageSink::spawn(&path, no_background_prune()).expect("spawn");

        sink.record(sample_record("2026-08-07T00:00:00+00:00"));
        let p = path.clone();
        wait_until("first row", move || !rows_of(&p).is_empty()).await;

        // 制造写失败：删表（后续写命令随之失败）。
        {
            let conn = sebas_db::conn::open(&path).unwrap();
            conn.execute_batch("DROP TABLE local_usage_records").unwrap();
        }
        for i in 0..8 {
            let mut rec = sample_record("2026-08-07T00:00:01+00:00");
            rec.latency_ms = i;
            sink.record(rec);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        let start = std::time::Instant::now();
        sink.record(sample_record("2026-08-07T00:00:02+00:00"));
        assert!(start.elapsed() < Duration::from_millis(200), "写失败后仍不阻塞");
    }

    #[tokio::test]
    async fn unusable_path_fails_at_startup_with_the_path_named() {
        let dir = tempdir().expect("tempdir");
        let blocker = dir.path().join("blocked");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let path = blocker.join("usage_local.db");

        let err = match LocalUsageSink::spawn(&path, LocalRetentionPolicy::default()) {
            Ok(_) => panic!("父目录被文件占位时不得启动成功"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("usage_local.db"),
            "错误必须点名库路径: {err}"
        );
    }

    // ---- 保留期双闸 ----

    #[tokio::test]
    async fn retention_prunes_records_older_than_the_window() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage_local.db");
        let policy = LocalRetentionPolicy {
            retention_days: 7,
            max_rows: 0,
            prune_interval_secs: 0,
        };
        let sink = LocalUsageSink::spawn(&path, policy).expect("spawn");

        let fresh = sample_record(&chrono::Utc::now().to_rfc3339());
        sink.record(sample_record("2000-01-01T00:00:00+00:00"));
        sink.record(fresh.clone());
        let p = path.clone();
        wait_until("two rows", move || rows_of(&p).len() >= 2).await;

        let writer = StateWriter::start(path.clone(), local_usage_tables()).unwrap();
        prune_once(writer.handle(), policy).await;

        let rows = rows_of(&path);
        assert_eq!(rows.len(), 1, "超期记录被清理");
        assert_eq!(rows[0].ts, fresh.ts, "窗口内的记录原样保留");
    }

    #[tokio::test]
    async fn retention_keeps_only_the_newest_rows_under_the_ceiling() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage_local.db");
        let policy = LocalRetentionPolicy {
            retention_days: 0,
            max_rows: 5,
            prune_interval_secs: 0,
        };
        let sink = LocalUsageSink::spawn(&path, policy).expect("spawn");

        for i in 0..12 {
            let mut rec = sample_record("2026-08-07T00:00:00+00:00");
            rec.model = Some(format!("m{i:02}"));
            sink.record(rec);
        }
        let p = path.clone();
        wait_until("12 rows", move || rows_of(&p).len() >= 12).await;

        let writer = StateWriter::start(path.clone(), local_usage_tables()).unwrap();
        prune_once(writer.handle(), policy).await;

        let rows = rows_of(&path);
        assert_eq!(rows.len(), 5, "清理后行数不超上限");
        let models: Vec<&str> = rows.iter().filter_map(|r| r.model.as_deref()).collect();
        assert_eq!(models, vec!["m07", "m08", "m09", "m10", "m11"]);
    }

    #[tokio::test]
    async fn zero_policy_prunes_nothing_and_background_pruner_runs_on_interval() {
        // 双闸全关 = 不清理。
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage_local.db");
        let policy = LocalRetentionPolicy {
            retention_days: 0,
            max_rows: 0,
            prune_interval_secs: 0,
        };
        let sink = LocalUsageSink::spawn(&path, policy).expect("spawn");
        sink.record(sample_record("2000-01-01T00:00:00+00:00"));
        let p = path.clone();
        wait_until("one row", move || !rows_of(&p).is_empty()).await;
        let writer = StateWriter::start(path.clone(), local_usage_tables()).unwrap();
        prune_once(writer.handle(), policy).await;
        assert_eq!(rows_of(&path).len(), 1, "双闸全关时不删任何记录");
        drop(writer);

        // 后台定期间隔：起 sink 投超期记录，等一个 tick 后它应被清掉。
        let dir2 = tempdir().expect("tempdir");
        let path2 = dir2.path().join("usage_local.db");
        let policy2 = LocalRetentionPolicy {
            retention_days: 1,
            max_rows: 0,
            prune_interval_secs: 1,
        };
        let sink2 = LocalUsageSink::spawn(&path2, policy2).expect("spawn");
        sink2.record(sample_record("2000-01-01T00:00:00+00:00"));
        let p2 = path2.clone();
        wait_until("row landed before prune", move || !rows_of(&p2).is_empty()).await;
        let p2 = path2.clone();
        wait_until("background pruner removed the aged row", move || {
            rows_of(&p2).is_empty()
        })
        .await;
    }

    // ---- 2.3：重启后仍在（本地历史不随进程消失） ----

    #[tokio::test]
    async fn rows_survive_a_process_restart() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage_local.db");
        {
            let sink = LocalUsageSink::spawn(&path, no_background_prune()).expect("spawn");
            // 记录落在查询窗口内（params() = 近 3 天、now = 2026-10-10）。
            sink.record(sample_record("2026-10-09T08:00:00+00:00"));
            sink.record(sample_record("2026-10-09T09:00:00+00:00"));
            let p = path.clone();
            wait_until("two rows", move || rows_of(&p).len() >= 2).await;
            // sink 随「进程退出」丢弃。
        }
        // 「重启」：新连接打开同一个库——此前落账的行原样可查。
        let writer = StateWriter::start(path.clone(), local_usage_tables()).unwrap();
        let ts = local_timeseries(
            Some(writer.handle()),
            params(),
            now_utc(),
        )
        .await
        .unwrap();
        assert_eq!(ts.totals.requests, 2, "重启后本地历史仍在聚合里");
    }

    // ---- 2.4：双算规避门控 ----

    #[test]
    fn native_gate_records_locally_only_for_direct_connections() {
        // ACP 恒本地（不走门控）；native 直连 = 本地。
        assert!(native_records_locally(None), "直连（无 router URL）→ 本地记");
        assert!(native_records_locally(Some("")), "空串视同未注入");
        assert!(
            !native_records_locally(Some("http://127.0.0.1:8787")),
            "注入 router URL → router 已记，本地不记"
        );
    }

    // ---- 4.1/4.2：source 三口径 ----

    async fn seeded_local_handle(dir: &Path, records: &[UsageRecord]) -> StateHandle {
        let writer =
            StateWriter::start(dir.join("usage_local.db"), local_usage_tables()).unwrap();
        let handle = writer.handle().clone();
        let rows: Vec<LocalUsageRow> = records.iter().map(LocalUsageRow::from).collect();
        handle
            .exec(move |conn| {
                let tx = sebas_db::conn::transaction(conn).map_err(|e| e.to_string())?;
                for row in &rows {
                    sebas_db::record::save(&tx, row).map_err(|e| e.to_string())?;
                }
                tx.commit().map_err(|e| e.to_string())
            })
            .await
            .expect("seed rows");
        handle
    }

    fn params() -> TimeseriesParams {
        TimeseriesParams {
            granularity: sebas_domain::usage::Granularity::Day,
            days: 3,
            tz_offset_min: 0,
        }
    }

    fn now_utc() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-10-10T10:00:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    /// 起 HTTP 服务的辅助：在一个已知端口上挂一个假 router admin 端点。
    async fn spawn_fake_router(
        status: u16,
        body: Value,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use axum::routing::get;
        let app = axum::Router::new().route(
            "/admin/usage/timeseries",
            get(move || async move { (axum::http::StatusCode::from_u16(status).unwrap(), axum::Json(body.clone())) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("127.0.0.1:{}", addr.port()), handle)
    }

    #[tokio::test]
    async fn source_local_aggregates_only_the_local_store() {
        let dir = tempdir().unwrap();
        let handle = seeded_local_handle(
            dir.path(),
            &[sample_record("2026-10-09T08:00:00+00:00")],
        )
        .await;
        let out = usage_timeseries_outcome(Some(&handle), None, usage_source::LOCAL, params(), now_utc()).await;
        let UsageQueryOutcome::Ok(payload) = out else {
            panic!("source=local 恒 200，got {out:?}");
        };
        let ts: Timeseries = serde_json::from_value(payload).unwrap();
        assert_eq!(ts.totals.requests, 1);
        assert_eq!(ts.totals.input_tokens, 10);
        assert!(ts.router_cause.is_none(), "local 口径不携带 router_cause");
        assert!(ts.buckets.iter().all(|b| b.models.iter().all(|m| m.by_source.is_none())));
    }

    #[tokio::test]
    async fn source_router_without_router_is_unreachable() {
        let out = usage_timeseries_outcome(None, None, usage_source::ROUTER, params(), now_utc()).await;
        let UsageQueryOutcome::RouterUnreachable { cause } = out else {
            panic!("router 未配置必须不可达，got {out:?}");
        };
        assert!(cause.starts_with("router_unreachable"), "{cause}");
    }

    #[tokio::test]
    async fn source_all_merges_local_and_router_without_double_counting() {
        let dir = tempdir().unwrap();
        let handle = seeded_local_handle(
            dir.path(),
            &[sample_record("2026-10-09T08:00:00+00:00")],
        )
        .await;
        // 假 router：同模型同桶 1 行（input 100）。
        let router_body = serde_json::json!({
            "granularity": "day",
            "days": 3,
            "tz_offset": 0,
            "buckets": [{
                "bucket": "2026-10-09",
                "models": [{
                    "model": "claude-sonnet",
                    "requests": 1,
                    "input_tokens": 100,
                    "output_tokens": 7,
                    "cache_read_tokens": 0,
                    "cache_creation_tokens": 0
                }]
            }],
            "totals": {
                "model": "total",
                "requests": 1,
                "input_tokens": 100,
                "output_tokens": 7,
                "cache_read_tokens": 0,
                "cache_creation_tokens": 0
            }
        });
        let (listen, server) = spawn_fake_router(200, router_body).await;
        let out = usage_timeseries_outcome(
            Some(&handle),
            Some(&listen),
            usage_source::ALL,
            params(),
            now_utc(),
        )
        .await;
        let UsageQueryOutcome::Ok(payload) = out else {
            panic!("source=all 恒 200，got {out:?}");
        };
        let ts: Timeseries = serde_json::from_value(payload).unwrap();
        let m = &ts.buckets.iter().find(|b| b.bucket == "2026-10-09").unwrap().models[0];
        assert_eq!(m.requests, 2, "合计 = 两源之和（本地 1 + router 1）");
        assert_eq!(m.input_tokens, 110);
        let split = m.by_source.as_ref().expect("all 口径必带小计");
        assert_eq!(split.local.requests, 1);
        assert_eq!(split.router.requests, 1);
        assert_eq!(split.local.input_tokens + split.router.input_tokens, m.input_tokens);
        assert!(ts.router_cause.is_none(), "router 在场不标缺席");
        server.abort();
    }

    #[tokio::test]
    async fn source_all_survives_router_absence_with_a_cause() {
        let dir = tempdir().unwrap();
        let handle = seeded_local_handle(
            dir.path(),
            &[sample_record("2026-10-09T08:00:00+00:00")],
        )
        .await;
        // router 未启用（listen = None）。
        let out = usage_timeseries_outcome(Some(&handle), None, usage_source::ALL, params(), now_utc()).await;
        let UsageQueryOutcome::Ok(payload) = out else {
            panic!("source=all 的 router 缺席必须仍 200，got {out:?}");
        };
        let ts: Timeseries = serde_json::from_value(payload).unwrap();
        assert_eq!(ts.totals.requests, 1, "本地数据照常返回");
        assert!(
            ts.router_cause
                .as_deref()
                .is_some_and(|c| c.starts_with("router_unreachable")),
            "缺席源必须带结构化 cause: {:?}",
            ts.router_cause
        );

        // router 已配置但端口无人听：同样 200 + cause。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let out = usage_timeseries_outcome(
            Some(&handle),
            Some(&format!("127.0.0.1:{port}")),
            usage_source::ALL,
            params(),
            now_utc(),
        )
        .await;
        let UsageQueryOutcome::Ok(payload) = out else {
            panic!("router 拒连不改变 all 的 200，got {out:?}");
        };
        let ts: Timeseries = serde_json::from_value(payload).unwrap();
        assert_eq!(ts.totals.requests, 1);
        assert!(ts.router_cause.is_some());
    }

    #[test]
    fn source_word_vocabulary_is_closed() {
        assert!(usage_source::is_valid("router"));
        assert!(usage_source::is_valid("local"));
        assert!(usage_source::is_valid("all"));
        assert!(!usage_source::is_valid("everything"));
        assert!(!usage_source::is_valid(""));
    }
}
