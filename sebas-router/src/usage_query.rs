//! usage 时序聚合查询（add-usage-statistics，design D2/D3/D4）。
//!
//! 把 [`crate::usage`] 逐行落库的 `usage_records` 聚合成按天/按小时的分模型
//! 时序统计——**纯查询**：零新表、零后台汇总任务、零写入路径改动。
//! 「次日后小时数据汇总为一天的总量」是查询口径（历史小时明细不再单独提供）。
//!
//! # 分层（tasks 1.1–1.3）
//!
//! - [`parse_params`]：查询参数解析 + 合法域 clamp（非法 400，越界收界）；
//! - [`window_start_utc`] / [`bucket_usage`] / [`timeseries`]：**桶切分纯函数**
//!   （输入记录集 + 参数 + now，输出零填充窗口；不经任何 I/O，单测直接钉）；
//! - [`query_timeseries`]：参数化 SQL（`ts` 窗口过滤走既有
//!   `idx_usage_records_ts` 索引）+ 纯函数分桶。分桶**不在 SQL** 里做：
//!   tz 偏移是分钟级（存在 +05:30 / +08:45 这类非整时区），SQL 侧
//!   `substr(ts,1,10)` 只能给 UTC 日期——偏移切桶在应用层按 RFC3339 解析后
//!   精确完成。
//!
//! # 口径（design D4）
//!
//! - 按 `model` 分组；`model` 为 NULL 计入 `(unknown)` 桶（总量诚实）；
//! - 每桶每模型：四类 token 求和 + 请求数；token 为 NULL（上游错误、解析
//!   失败）不计入 token 和、但计入请求数（SQL SUM 忽略 NULL 的同款语义在
//!   [`ModelUsage::add_record`] 复刻）；
//! - 聚合不按 `status` 过滤：成功请求才有 token 计数，失败行自然为零；
//! - 两粒度都**零填充**返回完整窗口（天 = 请求窗口的每个日期；小时 = 当天
//!   0–23 全 24 桶），折线图无需补点逻辑。

use std::fmt;

use chrono::{DateTime, Days, Duration, FixedOffset, NaiveDate, Timelike, Utc};
use serde::Serialize;

use sebas_db::record::Record;
use sebas_db::writer::StateHandle;

use crate::usage::UsageRow;

/// `model` 为 NULL 的记录计入的桶名（D4：不悄悄丢行，总量诚实）。
pub const UNKNOWN_MODEL: &str = "(unknown)";

/// 天粒度窗口缺省（design D2：默认近 14 天）。
pub const DAYS_DEFAULT: u32 = 14;
/// 天粒度窗口下界（task 1.3：clamp 1–30）。
pub const DAYS_MIN: u32 = 1;
/// 天粒度窗口上界（task 1.3：clamp 1–30；design D2「窗口上限 30 天封顶」）。
pub const DAYS_MAX: u32 = 30;
/// `tz_offset` 合法域边界：±14h（design D3「clamp 到 ±14h 合法域」）。
pub const TZ_OFFSET_MAX_MIN: i32 = 14 * 60;

/// 聚合粒度（wire 词表：`day` | `hour`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    /// 按天：请求窗口内每个日期一个桶（零填充）。
    Day,
    /// 按小时：仅当天（按请求偏移的「今天」）0–23 全 24 桶。
    Hour,
}

impl Granularity {
    /// wire 拼写。
    pub fn as_str(self) -> &'static str {
        match self {
            Granularity::Day => "day",
            Granularity::Hour => "hour",
        }
    }

    /// 解析 wire 取值；未知拼写返回 `None`（调用方 400）。
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "day" => Some(Granularity::Day),
            "hour" => Some(Granularity::Hour),
            _ => None,
        }
    }
}

/// 参数解析错误（task 1.3：非法 400；越界数值 clamp 而非拒绝）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamError {
    /// `granularity` 不是 `day`/`hour`。
    UnknownGranularity(String),
    /// `days` 不是非负整数。
    BadDays(String),
    /// `tz_offset` 不是整数。
    BadTzOffset(String),
}

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamError::UnknownGranularity(v) => {
                write!(f, "invalid granularity {v:?}: expected \"day\" or \"hour\"")
            }
            ParamError::BadDays(v) => write!(f, "invalid days {v:?}: expected a number"),
            ParamError::BadTzOffset(v) => write!(f, "invalid tz_offset {v:?}: expected minutes as a number"),
        }
    }
}

/// 已解析并收界的聚合参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeseriesParams {
    /// 聚合粒度。
    pub granularity: Granularity,
    /// 天粒度窗口（1–30；hour 忽略）。缺省 [`DAYS_DEFAULT`]。
    pub days: u32,
    /// 分钟东偏（clamp ±[`TZ_OFFSET_MAX_MIN`]；缺省 0 = UTC）。
    pub tz_offset_min: i32,
}

/// 解析查询参数（task 1.3）：
///
/// - `granularity`：缺省 `day`；非法拼写 → [`ParamError::UnknownGranularity`]；
/// - `days`：缺省 14；非数字 → [`ParamError::BadDays`]；越界 clamp 1–30
///   （hour 粒度忽略该参数——不解析不报错，只有当请求带了非法数字时仍 400，
///   与 granularity 的处理对称且可预期）；
/// - `tz_offset`：缺省 0；非数字 → [`ParamError::BadTzOffset`]；越界 clamp
///   ±840 分钟。
pub fn parse_params(
    granularity: Option<&str>,
    days: Option<&str>,
    tz_offset: Option<&str>,
) -> Result<TimeseriesParams, ParamError> {
    let granularity = match granularity {
        None | Some("") => Granularity::Day,
        Some(g) => Granularity::from_wire(g)
            .ok_or_else(|| ParamError::UnknownGranularity(g.to_string()))?,
    };
    let days = match days {
        None | Some("") => DAYS_DEFAULT,
        Some(d) => {
            let n: i64 = d
                .parse()
                .map_err(|_| ParamError::BadDays(d.to_string()))?;
            n.clamp(DAYS_MIN as i64, DAYS_MAX as i64) as u32
        }
    };
    let tz_offset_min = match tz_offset {
        None | Some("") => 0,
        Some(t) => {
            let n: i64 = t
                .parse()
                .map_err(|_| ParamError::BadTzOffset(t.to_string()))?;
            n.clamp(-(TZ_OFFSET_MAX_MIN as i64), TZ_OFFSET_MAX_MIN as i64) as i32
        }
    };
    Ok(TimeseriesParams {
        granularity,
        days,
        tz_offset_min,
    })
}

impl TimeseriesParams {
    /// 请求下传的时区偏移（分钟东偏已 clamp，构造必不失败）。
    fn offset(self) -> FixedOffset {
        FixedOffset::east_opt(self.tz_offset_min * 60)
            .expect("tz_offset clamp 到 ±840 分钟内，FixedOffset 必然合法")
    }

    /// 窗口起点（UTC，含）。天粒度 = 按偏移的「今天」回退 `days-1` 天的
    /// 当地 00:00；小时粒度 = 当地今天 00:00。换算回 UTC 后交给 SQL 做
    /// `ts >= ?` 索引过滤（`ts` 为 RFC3339 UTC 串，字典序 = 时间序）。
    pub fn window_start_utc(self, now_utc: DateTime<Utc>) -> DateTime<Utc> {
        let local_now = now_utc.with_timezone(&self.offset());
        let today = local_now.date_naive();
        let back = match self.granularity {
            Granularity::Day => u64::from(self.days.max(1)) - 1,
            Granularity::Hour => 0,
        };
        let start_local_date: NaiveDate =
            today - Days::new(back);
        let start_local = start_local_date
            .and_hms_opt(0, 0, 0)
            .expect("当地午夜必然存在");
        // 当地时间 = UTC + 偏移 → UTC = 当地 - 偏移。
        DateTime::from_naive_utc_and_offset(
            start_local - Duration::minutes(i64::from(self.tz_offset_min)),
            Utc,
        )
    }
}

/// 一桶内单模型的用量（D4：四类明细 + 请求数）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelUsage {
    /// 操作员视角的路由模型名；NULL 记录为 `(unknown)`。
    pub model: String,
    /// 落在该桶该模型上的请求行数（token 未观测的行也计入）。
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
}

impl ModelUsage {
    fn empty(model: impl Into<String>) -> Self {
        ModelUsage {
            model: model.into(),
            requests: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        }
    }

    /// 累加一行记录。`None` token（未观测）不计入 token 和、但计入请求数
    /// （D4 / spec 场景「unobserved tokens contribute nothing」）。
    fn add_record(&mut self, row: &UsageRow) {
        self.requests += 1;
        self.input_tokens += nonneg(row.input_tokens);
        self.output_tokens += nonneg(row.output_tokens);
        self.cache_read_tokens += nonneg(row.cache_read_tokens);
        self.cache_creation_tokens += nonneg(row.cache_creation_tokens);
    }
}

/// token 列读数：NULL（未观测）计 0；防御性把负值也当 0（token 计数恒非负，
/// 越界值不进统计面）。
fn nonneg(v: Option<i64>) -> u64 {
    v.unwrap_or(0).max(0) as u64
}

/// 一个时间桶：桶标签 + 按模型分列的用量（模型名字典序）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimeseriesBucket {
    /// 天粒度 = `YYYY-MM-DD`（按请求偏移的当地日期）；小时粒度 = 当地小时
    /// `00`–`23` 两位。
    pub bucket: String,
    /// 该桶内的分模型用量（按模型名字典序，输出确定）。
    pub models: Vec<ModelUsage>,
}

/// 聚合结果：零填充完整窗口 + 全窗合计（前端汇总数字的直接来源）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Timeseries {
    /// 回显粒度（`day` | `hour`）。
    pub granularity: String,
    /// 回显生效窗口（天粒度；hour 恒为 1——它只有今天）。
    pub days: u32,
    /// 回显生效偏移（clamp 后）。
    pub tz_offset: i32,
    /// 时间升序的完整桶窗口（无数据桶为空 models 数组，零填充）。
    pub buckets: Vec<TimeseriesBucket>,
    /// 全窗合计（各桶各模型之和；与 buckets 逐项一致，前端汇总不必重算）。
    /// 复用 [`ModelUsage`] 形状，`model` 恒为 `"total"`。
    pub totals: ModelUsage,
}

/// 窗口桶标签（时间升序）：天 = 最近 `days` 个当地日期；小时 = `00`–`23`。
pub fn bucket_labels(params: TimeseriesParams, now_utc: DateTime<Utc>) -> Vec<String> {
    let local_today = now_utc.with_timezone(&params.offset()).date_naive();
    match params.granularity {
        Granularity::Day => (0..params.days.max(1))
            .map(|i| {
                let date = local_today - Days::new(u64::from(params.days.max(1)) - 1 - i as u64);
                date.format("%Y-%m-%d").to_string()
            })
            .collect(),
        Granularity::Hour => (0..24).map(|h| format!("{h:02}")).collect(),
    }
}

/// 单条记录的桶标签（按请求偏移切分；解析失败 → `None`，防御性跳过——
/// `ts` 是 router 自己写的 RFC3339 串，正常路径不会失败）。
pub fn bucket_of_ts(ts: &str, params: TimeseriesParams) -> Option<String> {
    let t = DateTime::parse_from_rfc3339(ts).ok()?;
    Some(bucket_of_instant(&t, params))
}

/// 已解析时刻的桶标签（`bucket_of_ts` 的解析一次复用形态）。
fn bucket_of_instant(t: &DateTime<FixedOffset>, params: TimeseriesParams) -> String {
    let local = t.with_timezone(&params.offset());
    match params.granularity {
        Granularity::Day => local.format("%Y-%m-%d").to_string(),
        Granularity::Hour => format!("{:02}", local.hour()),
    }
}

/// 桶切分纯函数（task 1.1）：输入记录集 + 参数 + now，输出零填充窗口的
/// 桶结构。窗口外记录被丢弃（SQL 侧已按窗口过滤，这里是同一口径的兜底：
/// 小时粒度只有小时标签，日期维度必须在这里真正按窗口起点剔除）。
pub fn bucket_usage(
    records: &[UsageRow],
    params: TimeseriesParams,
    now_utc: DateTime<Utc>,
) -> Vec<TimeseriesBucket> {
    let labels = bucket_labels(params, now_utc);
    let start = params.window_start_utc(now_utc);
    // (label, model) → 累加器；BTreeMap 保证模型输出字典序确定。
    let mut acc: std::collections::BTreeMap<(String, String), ModelUsage> =
        std::collections::BTreeMap::new();
    for row in records {
        let Ok(t) = DateTime::parse_from_rfc3339(&row.ts) else {
            continue;
        };
        if t.to_utc() < start {
            continue;
        }
        let label = bucket_of_instant(&t, params);
        let model = row.model.clone().unwrap_or_else(|| UNKNOWN_MODEL.to_string());
        acc.entry((label, model))
            .or_insert_with(|| ModelUsage::empty(String::new()))
            .add_record(row);
    }
    labels
        .into_iter()
        .map(|label| {
            let models: Vec<ModelUsage> = acc
                .iter()
                .filter(|((l, _), _)| *l == label)
                .map(|((_, m), usage)| ModelUsage {
                    model: m.clone(),
                    ..usage.clone()
                })
                .collect();
            TimeseriesBucket { bucket: label, models }
        })
        .collect()
}

/// 聚合 + 全窗合计（admin 端点的响应体）。
pub fn timeseries(
    records: &[UsageRow],
    params: TimeseriesParams,
    now_utc: DateTime<Utc>,
) -> Timeseries {
    let buckets = bucket_usage(records, params, now_utc);
    let mut totals = ModelUsage::empty("total");
    for bucket in &buckets {
        for usage in &bucket.models {
            totals.requests += usage.requests;
            totals.input_tokens += usage.input_tokens;
            totals.output_tokens += usage.output_tokens;
            totals.cache_read_tokens += usage.cache_read_tokens;
            totals.cache_creation_tokens += usage.cache_creation_tokens;
        }
    }
    Timeseries {
        granularity: params.granularity.as_str().to_string(),
        days: if params.granularity == Granularity::Hour {
            1
        } else {
            params.days
        },
        tz_offset: params.tz_offset_min,
        buckets,
        totals,
    }
}

/// 聚合查询（task 1.2）：参数化 SQL 按 `ts` 窗口过滤（走既有
/// `idx_usage_records_ts` 索引），行级取回后交给纯函数分桶。只读 SELECT
/// 经 [`StateHandle::exec`] 在单写线程串行执行——与写入/清理共用一条命令
/// 队列，毫秒级，绝不影响转发路径（D7）。
pub async fn query_timeseries(
    handle: &StateHandle,
    params: TimeseriesParams,
    now_utc: DateTime<Utc>,
) -> Result<Timeseries, String> {
    let start = params.window_start_utc(now_utc).to_rfc3339();
    let rows = handle
        .exec(move |conn| -> Result<Vec<UsageRow>, String> {
            let sql = format!(
                "SELECT {} FROM usage_records WHERE ts >= ?1",
                <UsageRow as Record>::COLUMNS.join(", ")
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([&start], UsageRow::from_row)
                .map_err(|e| e.to_string())?
                .collect::<sebas_db::rusqlite::Result<Vec<UsageRow>>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        })
        .await?;
    Ok(timeseries(&rows, params, now_utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ts: &str, model: Option<&str>, input: Option<i64>, output: Option<i64>) -> UsageRow {
        UsageRow {
            id: None,
            key: String::new(),
            protocol: "anthropic".into(),
            model: model.map(str::to_string),
            provider: "anthropic".into(),
            upstream_model: None,
            status: 200,
            latency_ms: 1,
            ttft_ms: None,
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            error: None,
            ts: ts.into(),
        }
    }

    fn day_params(days: u32, tz: i32) -> TimeseriesParams {
        TimeseriesParams {
            granularity: Granularity::Day,
            days,
            tz_offset_min: tz,
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-29T10:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc)
    }

    // ---------------- task 1.3 参数口径 ----------------

    #[test]
    fn granularity_must_be_day_or_hour() {
        assert_eq!(
            parse_params(Some("week"), None, None),
            Err(ParamError::UnknownGranularity("week".into()))
        );
        assert_eq!(
            parse_params(None, None, None).unwrap().granularity,
            Granularity::Day,
            "缺省 day"
        );
        assert_eq!(
            parse_params(Some("hour"), None, None).unwrap().granularity,
            Granularity::Hour
        );
    }

    #[test]
    fn days_default_clamped_and_ignored_for_hour() {
        let p = parse_params(None, None, None).unwrap();
        assert_eq!(p.days, DAYS_DEFAULT, "缺省 14");
        // 越界 clamp 1–30（task 1.3），非数字才 400。
        assert_eq!(parse_params(None, Some("0"), None).unwrap().days, 1);
        assert_eq!(parse_params(None, Some("999"), None).unwrap().days, 30);
        assert_eq!(parse_params(None, Some("7"), None).unwrap().days, 7);
        assert_eq!(
            parse_params(None, Some("abc"), None),
            Err(ParamError::BadDays("abc".into()))
        );
        // hour 忽略 days：不因缺省/越界值改变粒度语义（仍然解析合法性）。
        let hour = parse_params(Some("hour"), Some("999"), None).unwrap();
        assert_eq!(hour.granularity, Granularity::Hour);
        assert_eq!(hour.days, 30, "解析照常 clamp，粒度不因 days 改变");
    }

    #[test]
    fn tz_offset_defaults_to_zero_and_clamps() {
        assert_eq!(parse_params(None, None, None).unwrap().tz_offset_min, 0);
        assert_eq!(parse_params(None, None, Some("480")).unwrap().tz_offset_min, 480);
        assert_eq!(
            parse_params(None, None, Some("2000")).unwrap().tz_offset_min,
            TZ_OFFSET_MAX_MIN,
            "越界 clamp +840"
        );
        assert_eq!(
            parse_params(None, None, Some("-2000")).unwrap().tz_offset_min,
            -TZ_OFFSET_MAX_MIN,
            "越界 clamp -840"
        );
        assert_eq!(
            parse_params(None, None, Some("+5.5")),
            Err(ParamError::BadTzOffset("+5.5".into()))
        );
    }

    // ---------------- 窗口与桶切分（task 1.1 / spec 场景） ----------------

    #[test]
    fn utc_23_00_lands_on_the_next_day_for_utc_plus_8() {
        let params = day_params(3, 480);
        // UTC 2026-09-29T23:00 = +8 的 2026-09-30T07:00 → 落 09-30 桶。
        // now 取在该记录之后（真实时序：记录落库先于查询），此时 +8 的
        // 「今天」已是 09-30，09-30 在窗口内。
        let now = DateTime::parse_from_rfc3339("2026-09-29T23:30:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let rows = [row("2026-09-29T23:00:00+00:00", Some("m"), Some(10), None)];
        let buckets = bucket_usage(&rows, params, now);
        let hit = buckets.iter().find(|b| b.bucket == "2026-09-30").unwrap();
        assert_eq!(hit.models.len(), 1);
        assert_eq!(hit.models[0].input_tokens, 10);
        // 窗口内更早的日期照旧零填充存在。
        assert!(buckets.iter().any(|b| b.bucket == "2026-09-28"));
        // 窗口外（更早）的记录不进任何桶。
        let old = [row("2026-09-01T00:00:00+00:00", Some("m"), Some(1), None)];
        let buckets = bucket_usage(&old, params, now);
        assert!(buckets.iter().all(|b| b.models.is_empty()));
    }

    #[test]
    fn two_models_aggregate_into_separate_day_groups_with_zero_fill() {
        let params = day_params(3, 0);
        let rows = [
            row("2026-09-28T08:00:00+00:00", Some("claude-sonnet"), Some(10), Some(50)),
            row("2026-09-28T09:00:00+00:00", Some("gpt-4o-mini"), Some(5), Some(8)),
            row("2026-09-28T09:30:00+00:00", Some("claude-sonnet"), Some(1), Some(2)),
        ];
        let buckets = bucket_usage(&rows, params, now());
        assert_eq!(buckets.len(), 3, "窗口逐日零填充");
        assert_eq!(buckets[0].bucket, "2026-09-27");
        assert!(buckets[0].models.is_empty(), "无数据桶零填充");
        let day28 = &buckets[1];
        assert_eq!(day28.bucket, "2026-09-28");
        assert_eq!(day28.models.len(), 2, "同日双模型分列");
        // 字典序：claude-sonnet 在前。
        assert_eq!(day28.models[0].model, "claude-sonnet");
        assert_eq!(day28.models[0].requests, 2);
        assert_eq!(day28.models[0].input_tokens, 11);
        assert_eq!(day28.models[0].output_tokens, 52);
        assert_eq!(day28.models[1].model, "gpt-4o-mini");
        assert_eq!(day28.models[1].input_tokens, 5);
        assert_eq!(day28.models[1].output_tokens, 8);
    }

    #[test]
    fn unobserved_tokens_contribute_nothing_but_count_as_requests() {
        let params = day_params(1, 0);
        let rows = [
            row("2026-09-29T01:00:00+00:00", Some("m"), Some(10), Some(5)),
            row("2026-09-29T02:00:00+00:00", Some("m"), None, None),
        ];
        let ts = timeseries(&rows, params, now());
        let models = &ts.buckets[0].models;
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].requests, 2, "失败行计入请求数");
        assert_eq!(models[0].input_tokens, 10, "None 不计入 token 和");
        assert_eq!(models[0].output_tokens, 5);
        assert_eq!(ts.totals.requests, 2);
        assert_eq!(ts.totals.input_tokens, 10);
    }

    #[test]
    fn null_model_lands_in_the_unknown_bucket() {
        let params = day_params(1, 0);
        let rows = [
            row("2026-09-29T01:00:00+00:00", None, Some(3), None),
            row("2026-09-29T01:30:00+00:00", Some("m"), Some(4), None),
        ];
        let buckets = bucket_usage(&rows, params, now());
        let models = &buckets[0].models;
        assert_eq!(models.len(), 2);
        // 字典序：(unknown) < m。
        assert_eq!(models[0].model, UNKNOWN_MODEL);
        assert_eq!(models[0].input_tokens, 3);
        assert_eq!(models[1].model, "m");
    }

    #[test]
    fn empty_window_stays_all_zero_filled() {
        let params = day_params(2, 0);
        let buckets = bucket_usage(&[], params, now());
        assert_eq!(buckets.len(), 2);
        assert!(buckets.iter().all(|b| b.models.is_empty()));
        // 全零窗口的合计也是零（折线图画平线的直接输入）。
        let ts = timeseries(&[], params, now());
        assert_eq!(ts.totals.requests, 0);
        assert_eq!(ts.buckets.len(), 2);
    }

    #[test]
    fn single_record_window_yields_a_single_point() {
        let params = day_params(7, 0);
        let rows = [row("2026-09-26T00:00:00+00:00", Some("m"), Some(1), None)];
        let buckets = bucket_usage(&rows, params, now());
        assert_eq!(buckets.len(), 7, "窗口仍全量零填充");
        let hit: Vec<_> = buckets.iter().filter(|b| !b.models.is_empty()).collect();
        assert_eq!(hit.len(), 1, "单点：只有一个桶带数据");
        assert_eq!(hit[0].bucket, "2026-09-26");
    }

    #[test]
    fn hour_granularity_covers_only_today_zero_to_23() {
        let params = TimeseriesParams {
            granularity: Granularity::Hour,
            days: 14,
            tz_offset_min: 480,
        };
        // UTC 2026-09-28T20:00 = +8 的 09-29T04:00 → 今天 04 时桶。
        let rows = [
            row("2026-09-28T20:00:00+00:00", Some("m"), Some(7), None),
            row("2026-09-27T23:00:00+00:00", Some("m"), Some(9), None), // 昨天（+8 的 09-28），必须排除
        ];
        let ts = timeseries(&rows, params, now());
        assert_eq!(ts.buckets.len(), 24, "0–23 全 24 桶");
        assert_eq!(ts.buckets[0].bucket, "00");
        assert_eq!(ts.buckets[23].bucket, "23");
        let hit: Vec<_> = ts.buckets.iter().filter(|b| !b.models.is_empty()).collect();
        assert_eq!(hit.len(), 1, "昨天的小时明细不再提供（spec 场景）");
        assert_eq!(hit[0].bucket, "04");
        assert_eq!(hit[0].models[0].input_tokens, 7);
    }

    #[test]
    fn window_start_respects_the_requested_offset() {
        // +8 的今天从当地 00:00 = UTC 前一天 16:00 起算。
        let params = day_params(1, 480);
        let start = params.window_start_utc(now());
        assert_eq!(
            start.to_rfc3339(),
            "2026-09-28T16:00:00+00:00",
            "UTC+8 的「一天」不从早 8 点起算（design D3）"
        );
        // UTC（偏移 0）窗口起点 = 当天 00:00 UTC。
        let utc_params = day_params(1, 0);
        assert_eq!(
            utc_params.window_start_utc(now()).to_rfc3339(),
            "2026-09-29T00:00:00+00:00"
        );
        // 14 天窗口：首日 = 当地今天 -13 天。
        let wide = day_params(14, 0);
        assert_eq!(
            wide.window_start_utc(now()).to_rfc3339(),
            "2026-09-16T00:00:00+00:00"
        );
    }
}
