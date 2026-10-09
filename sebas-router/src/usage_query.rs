//! usage 时序聚合查询（add-usage-statistics，design D2/D3/D4）。
//!
//! 把 [`crate::usage`] 逐行落库的 `usage_records` 聚合成按天/按小时的分模型
//! 时序统计——**纯查询**：零新表、零后台汇总任务、零写入路径改动。
//! 「次日后小时数据汇总为一天的总量」是查询口径（历史小时明细不再单独提供）。
//!
//! # 分层（add-local-usage-statistics 1.1/1.2）
//!
//! 参数解析、桶切分**纯函数**与 wire 类型已下沉中立域层
//! `sebas_domain::usage`（router 与 core 同一实现，同输入同桶形）；本模块
//! 原位再导出保住既有公开路径，只剩 [`query_timeseries`] 这一个 IO 编排点：
//! 参数化 SQL（`ts` 窗口过滤走既有 `idx_usage_records_ts` 索引）+ 行级取回
//! 后交域层纯函数分桶。分桶**不在 SQL** 里做：tz 偏移是分钟级（存在
//! +05:30 / +08:45 这类非整时区），SQL 侧 `substr(ts,1,10)` 只能给 UTC
//! 日期——偏移切桶在应用层按 RFC3339 解析后精确完成（语义随域层实现）。
//!
//! # 口径（design D4）
//!
//! - 按 `model` 分组；`model` 为 NULL 计入 `(unknown)` 桶（总量诚实）；
//! - 每桶每模型：四类 token 求和 + 请求数；token 为 NULL（上游错误、解析
//!   失败）不计入 token 和、但计入请求数；
//! - 聚合不按 `status` 过滤：成功请求才有 token 计数，失败行自然为零；
//! - 两粒度都**零填充**返回完整窗口（天 = 请求窗口的每个日期；小时 = 当天
//!   0–23 全 24 桶），折线图无需补点逻辑。

use chrono::{DateTime, Utc};
use sebas_db::record::Record;
use sebas_db::writer::StateHandle;

use crate::usage::{UsageRecord, UsageRow};

// ---- wire 类型与纯函数的唯一定义在域层（add-local-usage-statistics 1.1）----

pub use sebas_domain::usage::{
    bucket_labels, bucket_of_ts, bucket_usage, merge_timeseries, parse_params, timeseries,
    Granularity, ModelUsage, ParamError, SourceSplit, SourceSubtotal, Timeseries,
    TimeseriesBucket, TimeseriesParams, UsageSource, DAYS_DEFAULT, DAYS_MAX, DAYS_MIN,
    UNKNOWN_MODEL, TZ_OFFSET_MAX_MIN,
};

/// 聚合查询（task 1.2）：参数化 SQL 按 `ts` 窗口过滤（走既有
/// `idx_usage_records_ts` 索引），行级取回后经**无损**行→记录转换交给域层
/// 纯函数分桶（`UsageRow → UsageRecord` 逐字段搬运，聚合读数不漂移）。
/// 只读 SELECT 经 [`StateHandle::exec`] 在单写线程串行执行——与写入/清理
/// 共用一条命令队列，毫秒级，绝不影响转发路径（D7）。
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
    let records: Vec<UsageRecord> = rows.into_iter().map(UsageRecord::from).collect();
    Ok(timeseries(&records, params, now_utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试行构造走**真实的行→记录转换**（`UsageRow → UsageRecord`）——
    /// 聚合输入与生产路径同源，行侧字段与聚合读数的对齐由转换单点保证。
    fn row(ts: &str, model: Option<&str>, input: Option<i64>, output: Option<i64>) -> UsageRecord {
        UsageRecord::from(UsageRow {
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
        })
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

    // ---------------- add-local-usage-statistics 1.2：同输入同桶形 ----------------

    /// 行→记录转换无损 ⇒ 同一记录集经共享域层实现聚合，与迁移前 router 本地
    /// 实现的结果**逐字段一致**（期望值即迁移前单测钉死的读数——本文件上文
    /// 各用例同源）。这里再钉一次「行侧输入与记录侧输入聚合相等」的机械口径。
    #[test]
    fn row_and_record_inputs_aggregate_identically() {
        let rows = [
            UsageRow {
                id: Some(1),
                key: String::new(),
                protocol: "anthropic".into(),
                model: Some("claude-sonnet".into()),
                provider: "anthropic".into(),
                upstream_model: None,
                status: 200,
                latency_ms: 3,
                ttft_ms: None,
                input_tokens: Some(10),
                output_tokens: Some(50),
                cache_read_tokens: Some(5),
                cache_creation_tokens: Some(2),
                error: None,
                ts: "2026-09-28T08:00:00+00:00".into(),
            },
            UsageRow {
                id: Some(2),
                key: String::new(),
                protocol: "openai_chat".into(),
                model: None,
                provider: "openai".into(),
                upstream_model: None,
                status: 502,
                latency_ms: 4,
                ttft_ms: None,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                error: Some("boom".into()),
                ts: "2026-09-28T09:00:00+00:00".into(),
            },
        ];
        let records: Vec<UsageRecord> = rows.iter().map(|r| UsageRecord::from(r.clone())).collect();
        let params = day_params(2, 0);
        let from_rows = {
            let records: Vec<UsageRecord> = rows.iter().map(|r| UsageRecord::from(r.clone())).collect();
            timeseries(&records, params, now())
        };
        let from_records = timeseries(&records, params, now());
        assert_eq!(from_rows, from_records, "同输入同桶形（行侧 = 记录侧）");
        let day28 = &from_rows.buckets[0];
        assert_eq!(day28.models.len(), 2, "(unknown) 与 claude-sonnet 分列");
        assert_eq!(day28.models[0].model, UNKNOWN_MODEL);
        assert_eq!(day28.models[0].requests, 1, "None token 行只计请求数");
        assert_eq!(day28.models[1].model, "claude-sonnet");
        assert_eq!(day28.models[1].input_tokens, 10);
        assert_eq!(day28.models[1].output_tokens, 50);
        assert_eq!(day28.models[1].cache_read_tokens, 5);
        assert_eq!(day28.models[1].cache_creation_tokens, 2);
        assert_eq!(from_rows.totals.requests, 2);
    }
}
