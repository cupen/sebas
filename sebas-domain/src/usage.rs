//! usage 域类型（add-local-usage-statistics D3）：回合用量记录与时序聚合的
//! 请求/响应形状，**唯一定义**处——router 与 core 两侧共用，同输入同桶形。
//!
//! # 放置与准入
//!
//! ≥2 crate 需要（router 聚合 + core 本地聚合/落账）且角色中立 ✓。**持久行
//! 概念不在这里**（放置规则）：`usage.db` 的 `UsageRow` 归 sebas-router、
//! `usage_local.db` 的 `LocalUsageRow` 归 core 侧消费模块——各写入者的行
//! struct 仍归各自 crate，本层只承载对外/跨侧形状。
//!
//! # 兼容面
//!
//! `sebas_router::usage::UsageRecord` 与 `sebas_router::usage_query` 的既有
//! 公开路径改为原位再导出（wire 逐字不变）：`/admin/usage/timeseries` 的
//! 响应字段集合与取值域零变化；`ModelUsage::by_source` 与
//! `Timeseries::router_cause` 是 core 合并口径的**新增可选字段**
//! （serde 默认值 + 缺省不上 wire，协议演进规则 1）——router 单源自洽时
//! 恒为 `None`，老客户端看到的字节形状与迁移前逐字一致。

use std::fmt;

use chrono::{DateTime, Days, Duration, FixedOffset, NaiveDate, Timelike, Utc};
use serde::{Deserialize, Serialize};

/// 一次请求/回合的用量记录。`key` 恒为空（无 per-key 身份；绝不记 token 本体）。
/// `error` 留给路由侧失败（如 connect 502）；上游 4xx/5xx 不填 `error`（status
/// 字段承载其错误语义）。token 字段为 `None` 表示本次未观测到该计数（如
/// 解析失败、流被截断、或上游错误响应无 usage）。
///
/// router 侧原定义（persist-router-usage）原样提升进本层；本地落账
/// （local-usage-capture）复用同一形状——`protocol` 承载 `acp`/`native` 等
/// 执行体标签，`status` 承载回合终态。
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

/// 回合级 token 用量的中立形状（design D5：与 sebas-acp 的 `TurnUsage`
/// 同构——字段名逐字一致）。native 内核解析上游响应 usage 后以它随回合
/// 事件上报 core；逐字段 `None` = 该计数未观测（绝不全零冒充）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnTokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
}

/// 回合终态 → 记录 `status` 的约定值（HTTP 风格；router 侧语义对齐）。
pub const TURN_STATUS_FINISHED: u16 = 200;
/// 操作者取消的回合（中性收尾，非失败）。
pub const TURN_STATUS_CANCELLED: u16 = 499;
/// 失败的回合。
pub const TURN_STATUS_FAILED: u16 = 500;

/// 本地行 `protocol` 列的执行体标签（写入侧打标，查询面不解释）。
pub const PROTOCOL_ACP: &str = "acp";
pub const PROTOCOL_NATIVE: &str = "native";

impl TurnTokenUsage {
    /// 是否携带**任何** token 计数（`usage_reported` 门控的同源判定：
    /// 从未上报的回合不冒充已上报）。
    pub fn reports_any_tokens(&self) -> bool {
        self.input_tokens.is_some()
            || self.output_tokens.is_some()
            || self.cache_read_input_tokens.is_some()
            || self.cache_creation_input_tokens.is_some()
    }

    /// 折算成共享回合记录（design D5）：ts = 回合完成时刻（此刻），四类
    /// token 逐字段透传 `None`（未上报不冒充零），provider 留空（best-effort
    /// 上游名缺席时的诚实形态）、`key` 恒空（无 per-key 身份）。ACP 与
    /// native 两条落账路径共用，`protocol` 打执行体标签。
    pub fn into_turn_record(
        self,
        protocol: &str,
        status: u16,
        latency_ms: u64,
        error: Option<String>,
    ) -> UsageRecord {
        UsageRecord {
            ts: chrono::Utc::now().to_rfc3339(),
            key: String::new(),
            protocol: protocol.to_string(),
            model: self.model,
            provider: String::new(),
            upstream_model: None,
            status,
            latency_ms,
            ttft_ms: None,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_read_tokens: self.cache_read_input_tokens,
            cache_creation_tokens: self.cache_creation_input_tokens,
            error,
        }
    }

    /// 逐字段累加（`None` 与 `Some` 相加取 `Some` 侧；两侧都 `None` 保持
    /// `None`——「从未上报」不因累加变成「上报了 0」）。
    pub fn accumulate(&mut self, other: &TurnTokenUsage) {
        if other.model.is_some() {
            self.model = other.model.clone();
        }
        self.input_tokens = add_opt(self.input_tokens, other.input_tokens);
        self.output_tokens = add_opt(self.output_tokens, other.output_tokens);
        self.cache_read_input_tokens =
            add_opt(self.cache_read_input_tokens, other.cache_read_input_tokens);
        self.cache_creation_input_tokens = add_opt(
            self.cache_creation_input_tokens,
            other.cache_creation_input_tokens,
        );
    }

    /// 同一消息的**累积帧**合并（SSE 的 `message_start` / `message_delta`
    /// 携带的是同一消息的累计读数，不是增量）：`other` 的 `Some` 字段覆盖
    /// `self` 同名字段，`None` 保留原值。与 [`TurnTokenUsage::accumulate`]
    /// 的区别：后者求和（跨模型调用），本方法取最新（同消息跨帧）。
    pub fn merge_latest(&mut self, other: &TurnTokenUsage) {
        if other.model.is_some() {
            self.model = other.model.clone();
        }
        if other.input_tokens.is_some() {
            self.input_tokens = other.input_tokens;
        }
        if other.output_tokens.is_some() {
            self.output_tokens = other.output_tokens;
        }
        if other.cache_read_input_tokens.is_some() {
            self.cache_read_input_tokens = other.cache_read_input_tokens;
        }
        if other.cache_creation_input_tokens.is_some() {
            self.cache_creation_input_tokens = other.cache_creation_input_tokens;
        }
    }

    /// 从一份**响应体/流帧** JSON 解析 usage（design D5：Anthropic / OpenAI
    /// 两形状）。两家的字段名各自识别，缺失字段保持 `None`：
    ///
    /// - Anthropic：`input_tokens` / `output_tokens` /
    ///   `cache_read_input_tokens` / `cache_creation_input_tokens`；
    /// - OpenAI chat completions：`prompt_tokens` / `completion_tokens`
    ///   （缓存命中在 `prompt_tokens_details.cached_tokens`，映射到
    ///   cache_read）。
    ///
    /// 无 usage 字段 / 非 JSON 对象 → 全 `None`（调用方按「未观测」处理）。
    pub fn from_response_json(v: &serde_json::Value) -> Self {
        let Some(obj) = v.as_object() else {
            return TurnTokenUsage::default();
        };
        let get = |key: &str| obj.get(key).and_then(|x| x.as_u64());
        if get("input_tokens").is_some()
            || get("output_tokens").is_some()
            || get("cache_read_input_tokens").is_some()
            || get("cache_creation_input_tokens").is_some()
        {
            // Anthropic 形状（`message.usage` / `message_delta.usage`）。
            TurnTokenUsage {
                model: None,
                input_tokens: get("input_tokens"),
                output_tokens: get("output_tokens"),
                cache_read_input_tokens: get("cache_read_input_tokens"),
                cache_creation_input_tokens: get("cache_creation_input_tokens"),
            }
        } else {
            // OpenAI 形状（`usage` 对象）。
            let cached = obj
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|x| x.as_u64());
            TurnTokenUsage {
                model: None,
                input_tokens: get("prompt_tokens"),
                output_tokens: get("completion_tokens"),
                cache_read_input_tokens: cached,
                cache_creation_input_tokens: None,
            }
        }
    }
}

/// `Option<u64>` 求和：至少一侧 `Some` 才有和（两侧 `None` = 未观测）。
fn add_opt(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (None, None) => None,
        (x, None) | (None, x) => x,
        (Some(x), Some(y)) => Some(x.saturating_add(y)),
    }
}

/// `model` 为 NULL 的记录计入的桶名（usage-statistics D4：不悄悄丢行，总量诚实）。
pub const UNKNOWN_MODEL: &str = "(unknown)";

/// 天粒度窗口缺省（usage-statistics design D2：默认近 14 天）。
pub const DAYS_DEFAULT: u32 = 14;
/// 天粒度窗口下界（clamp 1–30）。
pub const DAYS_MIN: u32 = 1;
/// 天粒度窗口上界（clamp 1–30；「窗口上限 30 天封顶」）。
pub const DAYS_MAX: u32 = 30;
/// `tz_offset` 合法域边界：±14h（clamp 到 ±840 分钟合法域）。
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

/// 参数解析错误（非法 400；越界数值 clamp 而非拒绝）。
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
            ParamError::BadTzOffset(v) => {
                write!(f, "invalid tz_offset {v:?}: expected minutes as a number")
            }
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

/// 解析查询参数：
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
            let n: i64 = d.parse().map_err(|_| ParamError::BadDays(d.to_string()))?;
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
        let start_local_date: NaiveDate = today - Days::new(back);
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

/// 一个来源的小计（同 [`ModelUsage`] 的数值五项；不带 model 名）。
/// `#[serde(default)]` 逐字段：老载荷（无 by_source）反序列化补零。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSubtotal {
    #[serde(default)]
    pub requests: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
}

impl SourceSubtotal {
    fn add_model(&mut self, m: &ModelUsage) {
        self.requests += m.requests;
        self.input_tokens += m.input_tokens;
        self.output_tokens += m.output_tokens;
        self.cache_read_tokens += m.cache_read_tokens;
        self.cache_creation_tokens += m.cache_creation_tokens;
    }
}

/// 桶内单模型行的 per-source 小计（add-local-usage-statistics D4：`router` /
/// `local` 两个字段）。合并口径（`source=all`）的模型行携带两侧小计——写入侧
/// 单源规则保证合计 = 两源之和、零重算。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSplit {
    #[serde(default)]
    pub router: SourceSubtotal,
    #[serde(default)]
    pub local: SourceSubtotal,
}

impl SourceSplit {
    /// 累加一侧的模型行。
    pub fn add(&mut self, source: UsageSource, m: &ModelUsage) {
        match source {
            UsageSource::Router => self.router.add_model(m),
            UsageSource::Local => self.local.add_model(m),
        }
    }
}

/// 用量来源（add-local-usage-statistics D4）。一行记录恰好归一个源：
/// 经 router 的回合由 router 记，不经 router 的回合由 core 本地记。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageSource {
    Router,
    Local,
}

/// `source` 查询参数词表（add-local-usage-statistics D4：`/api/usage/
/// timeseries?source=`，core 通道帧与 webui 路由共用同一合法域）。缺省
/// [`usage_source::ALL`]（合并口径）。
pub mod usage_source {
    pub const ROUTER: &str = "router";
    pub const LOCAL: &str = "local";
    pub const ALL: &str = "all";

    /// 合法域判定（非法拼写由调用方 400）。
    pub fn is_valid(s: &str) -> bool {
        matches!(s, ROUTER | LOCAL | ALL)
    }
}

/// 一桶内单模型的用量（四类明细 + 请求数）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUsage {
    /// 操作员视角的路由模型名；NULL 记录为 `(unknown)`。
    pub model: String,
    /// 落在该桶该模型上的请求行数（token 未观测的行也计入）。
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    /// per-source 小计（add-local-usage-statistics D4）。**仅合并口径**
    ///（core 的 `source=all` 响应）携带；router 单源响应缺省不上 wire
    ///（缺省值不序列化——老客户端看到的字节形状零变化）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_source: Option<SourceSplit>,
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
            by_source: None,
        }
    }

    /// 累加一行记录。`None` token（未观测）不计入 token 和、但计入请求数
    ///（spec 场景「unobserved tokens contribute nothing」）。
    fn add_record(&mut self, row: &UsageRecord) {
        self.requests += 1;
        self.input_tokens += row.input_tokens.unwrap_or(0);
        self.output_tokens += row.output_tokens.unwrap_or(0);
        self.cache_read_tokens += row.cache_read_tokens.unwrap_or(0);
        self.cache_creation_tokens += row.cache_creation_tokens.unwrap_or(0);
    }
}

/// 一个时间桶：桶标签 + 按模型分列的用量（模型名字典序）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeseriesBucket {
    /// 天粒度 = `YYYY-MM-DD`（按请求偏移的当地日期）；小时粒度 = 当地小时
    /// `00`–`23` 两位。
    pub bucket: String,
    /// 该桶内的分模型用量（按模型名字典序，输出确定）。
    pub models: Vec<ModelUsage>,
}

/// 聚合结果：零填充完整窗口 + 全窗合计（前端汇总数字的直接来源）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// `source=all` 且 router 缺席时的结构化 cause（design D4：降级不拖垮
    /// ——本地数据照常返回，缺席源如实标注）。单源响应与 router 在场的
    /// 合并响应缺省不上 wire。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub router_cause: Option<String>,
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
/// `ts` 是写入方自己写的 RFC3339 串，正常路径不会失败）。
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

/// 桶切分纯函数：输入记录集 + 参数 + now，输出零填充窗口的桶结构。窗口外
/// 记录被丢弃（SQL 侧已按窗口过滤，这里是同一口径的兜底：小时粒度只有小时
/// 标签，日期维度必须在这里真正按窗口起点剔除）。
pub fn bucket_usage(
    records: &[UsageRecord],
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

/// 聚合 + 全窗合计（聚合端点的响应体）。
pub fn timeseries(
    records: &[UsageRecord],
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
        router_cause: None,
    }
}

/// 合并两个同参聚合（add-local-usage-statistics D4 / task 4.2）：桶内同模型
/// 行相加并携带 per-source 小计（`local` + `router`），合计同步。两侧必须以
/// 相同 `params`/`now` 聚合（调用方契约）——桶标签集合因此一致，标签缺失侧
/// 按零处理（防御：不 panic）。`router_cause` 原样带到合并结果（缺席源标注）。
pub fn merge_timeseries(local: &Timeseries, router: &Timeseries) -> Timeseries {
    let mut buckets: Vec<TimeseriesBucket> = Vec::with_capacity(local.buckets.len());
    // 桶标签按 local 序（零填充窗口，两侧同参时逐位相等）。
    let labels: Vec<&str> = local.buckets.iter().map(|b| b.bucket.as_str()).collect();
    for label in labels {
        let lb = local.buckets.iter().find(|b| b.bucket == label);
        let rb = router.buckets.iter().find(|b| b.bucket == label);
        // (model) → (合计, per-source)。BTreeMap 保持模型名字典序输出。
        let mut models: std::collections::BTreeMap<String, (ModelUsage, SourceSplit)> =
            std::collections::BTreeMap::new();
        for m in lb.iter().flat_map(|b| &b.models) {
            let e = models
                .entry(m.model.clone())
                .or_insert_with(|| (ModelUsage::empty(String::new()), SourceSplit::default()));
            e.0.add_record_total(m);
            e.1.add(UsageSource::Local, m);
        }
        for m in rb.iter().flat_map(|b| &b.models) {
            let e = models
                .entry(m.model.clone())
                .or_insert_with(|| (ModelUsage::empty(String::new()), SourceSplit::default()));
            e.0.add_record_total(m);
            e.1.add(UsageSource::Router, m);
        }
        let merged: Vec<ModelUsage> = models
            .into_iter()
            .map(|(model, (total, split))| ModelUsage {
                by_source: Some(split),
                model,
                ..total
            })
            .collect();
        buckets.push(TimeseriesBucket {
            bucket: label.to_string(),
            models: merged,
        });
    }
    let mut totals = ModelUsage::empty("total");
    let mut total_split = SourceSplit::default();
    for b in &buckets {
        for m in &b.models {
            totals.requests += m.requests;
            totals.input_tokens += m.input_tokens;
            totals.output_tokens += m.output_tokens;
            totals.cache_read_tokens += m.cache_read_tokens;
            totals.cache_creation_tokens += m.cache_creation_tokens;
            if let Some(s) = &m.by_source {
                total_split.router = sum_subtotal(total_split.router, s.router);
                total_split.local = sum_subtotal(total_split.local, s.local);
            }
        }
    }
    totals.by_source = Some(total_split);
    Timeseries {
        granularity: local.granularity.clone(),
        days: local.days,
        tz_offset: local.tz_offset,
        buckets,
        totals,
        router_cause: router.router_cause.clone(),
    }
}

/// ModelUsage 的**聚合值**累加（与 `add_record` 相对：这里累加的是已聚合的
/// 行，非单条记录）。`by_source` 不在此合并——小计由 [`SourceSplit`] 侧账。
trait AddAggregated {
    fn add_record_total(&mut self, other: &ModelUsage);
}

impl AddAggregated for ModelUsage {
    fn add_record_total(&mut self, other: &ModelUsage) {
        self.requests += other.requests;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
    }
}

fn sum_subtotal(a: SourceSubtotal, b: SourceSubtotal) -> SourceSubtotal {
    SourceSubtotal {
        requests: a.requests + b.requests,
        input_tokens: a.input_tokens + b.input_tokens,
        output_tokens: a.output_tokens + b.output_tokens,
        cache_read_tokens: a.cache_read_tokens + b.cache_read_tokens,
        cache_creation_tokens: a.cache_creation_tokens + b.cache_creation_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(ts: &str, model: Option<&str>, input: Option<u64>, output: Option<u64>) -> UsageRecord {
        UsageRecord {
            ts: ts.into(),
            key: String::new(),
            protocol: "acp".into(),
            model: model.map(str::to_string),
            provider: String::new(),
            upstream_model: None,
            status: 200,
            latency_ms: 1,
            ttft_ms: None,
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            error: None,
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-29T10:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn day_params(days: u32, tz: i32) -> TimeseriesParams {
        TimeseriesParams {
            granularity: Granularity::Day,
            days,
            tz_offset_min: tz,
        }
    }

    // ---- TurnTokenUsage：同构形状与累加门控 ----

    #[test]
    fn turn_token_usage_accumulates_without_fabricating_zeroes() {
        let mut acc = TurnTokenUsage::default();
        assert!(!acc.reports_any_tokens(), "从未上报 ≠ 已上报");
        acc.accumulate(&TurnTokenUsage {
            model: Some("m".into()),
            input_tokens: Some(10),
            ..Default::default()
        });
        acc.accumulate(&TurnTokenUsage {
            output_tokens: Some(5),
            ..Default::default()
        });
        assert_eq!(acc.input_tokens, Some(10));
        assert_eq!(acc.output_tokens, Some(5));
        assert!(acc.reports_any_tokens());
        assert_eq!(acc.model.as_deref(), Some("m"), "model 取最近一次上报");
        // 两侧都 None 的字段保持 None（不冒充 0）。
        assert_eq!(acc.cache_read_input_tokens, None);
    }

    #[test]
    fn turn_token_usage_wire_round_trip_omits_unobserved() {
        let u = TurnTokenUsage {
            model: Some("claude-sonnet".into()),
            input_tokens: Some(7),
            ..Default::default()
        };
        let v = serde_json::to_value(&u).unwrap();
        assert!(!v.as_object().unwrap().contains_key("output_tokens"));
        let back: TurnTokenUsage = serde_json::from_value(v).unwrap();
        assert_eq!(back, u);
    }

    // ---- 聚合纯函数（router 侧既有口径的同构复刻，两侧同实现） ----

    #[test]
    fn two_models_aggregate_into_separate_day_groups_with_zero_fill() {
        let params = day_params(3, 0);
        let rows = [
            record("2026-09-28T08:00:00+00:00", Some("claude-sonnet"), Some(10), Some(50)),
            record("2026-09-28T09:00:00+00:00", Some("gpt-4o-mini"), Some(5), Some(8)),
            record("2026-09-28T09:30:00+00:00", Some("claude-sonnet"), Some(1), Some(2)),
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
        // 单源聚合不携带 per-source 小计（router wire 零漂移）。
        assert!(day28.models[0].by_source.is_none());
        assert!(bucket_usage(&rows, params, now())[1].models[0].by_source.is_none());
    }

    #[test]
    fn unobserved_tokens_contribute_nothing_but_count_as_requests() {
        let params = day_params(1, 0);
        let rows = [
            record("2026-09-29T01:00:00+00:00", Some("m"), Some(10), Some(5)),
            record("2026-09-29T02:00:00+00:00", Some("m"), None, None),
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
            record("2026-09-29T01:00:00+00:00", None, Some(3), None),
            record("2026-09-29T01:30:00+00:00", Some("m"), Some(4), None),
        ];
        let buckets = bucket_usage(&rows, params, now());
        let models = &buckets[0].models;
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].model, UNKNOWN_MODEL);
        assert_eq!(models[0].input_tokens, 3);
        assert_eq!(models[1].model, "m");
    }

    #[test]
    fn utc_23_00_lands_on_the_next_day_for_utc_plus_8() {
        let params = day_params(3, 480);
        let now = DateTime::parse_from_rfc3339("2026-09-29T23:30:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let rows = [record("2026-09-29T23:00:00+00:00", Some("m"), Some(10), None)];
        let buckets = bucket_usage(&rows, params, now);
        let hit = buckets.iter().find(|b| b.bucket == "2026-09-30").unwrap();
        assert_eq!(hit.models.len(), 1);
        assert_eq!(hit.models[0].input_tokens, 10);
        assert!(buckets.iter().any(|b| b.bucket == "2026-09-28"));
        let old = [record("2026-09-01T00:00:00+00:00", Some("m"), Some(1), None)];
        let buckets = bucket_usage(&old, params, now);
        assert!(buckets.iter().all(|b| b.models.is_empty()));
    }

    #[test]
    fn hour_granularity_covers_only_today_zero_to_23() {
        let params = TimeseriesParams {
            granularity: Granularity::Hour,
            days: 14,
            tz_offset_min: 480,
        };
        let rows = [
            record("2026-09-28T20:00:00+00:00", Some("m"), Some(7), None),
            record("2026-09-27T23:00:00+00:00", Some("m"), Some(9), None),
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
        let params = day_params(1, 480);
        let start = params.window_start_utc(now());
        assert_eq!(
            start.to_rfc3339(),
            "2026-09-28T16:00:00+00:00",
            "UTC+8 的「一天」不从早 8 点起算"
        );
        let utc_params = day_params(1, 0);
        assert_eq!(
            utc_params.window_start_utc(now()).to_rfc3339(),
            "2026-09-29T00:00:00+00:00"
        );
        let wide = day_params(14, 0);
        assert_eq!(
            wide.window_start_utc(now()).to_rfc3339(),
            "2026-09-16T00:00:00+00:00"
        );
    }

    #[test]
    fn param_parsing_defaults_clamps_and_rejects() {
        assert_eq!(
            parse_params(Some("week"), None, None),
            Err(ParamError::UnknownGranularity("week".into()))
        );
        assert_eq!(parse_params(None, None, None).unwrap().granularity, Granularity::Day);
        assert_eq!(parse_params(None, None, None).unwrap().days, DAYS_DEFAULT);
        assert_eq!(parse_params(None, Some("0"), None).unwrap().days, 1);
        assert_eq!(parse_params(None, Some("999"), None).unwrap().days, 30);
        assert_eq!(
            parse_params(None, Some("abc"), None),
            Err(ParamError::BadDays("abc".into()))
        );
        assert_eq!(parse_params(None, None, Some("2000")).unwrap().tz_offset_min, TZ_OFFSET_MAX_MIN);
        assert_eq!(
            parse_params(None, None, Some("-2000")).unwrap().tz_offset_min,
            -TZ_OFFSET_MAX_MIN
        );
        assert_eq!(
            parse_params(None, None, Some("+5.5")),
            Err(ParamError::BadTzOffset("+5.5".into()))
        );
    }

    // ---- 合并（task 4.2：同模型同桶合计 = 两源之和，per-source 小计在场） ----

    #[test]
    fn merge_sums_both_sources_without_double_counting() {
        let params = day_params(2, 0);
        let local = timeseries(
            &[
                record("2026-09-28T08:00:00+00:00", Some("m"), Some(10), Some(4)),
                record("2026-09-28T09:00:00+00:00", Some("m"), Some(1), None),
            ],
            params,
            now(),
        );
        let mut router = timeseries(
            &[record("2026-09-28T08:30:00+00:00", Some("m"), Some(100), Some(50))],
            params,
            now(),
        );
        router.router_cause = None;
        let merged = merge_timeseries(&local, &router);

        let day28 = merged.buckets.iter().find(|b| b.bucket == "2026-09-28").unwrap();
        assert_eq!(day28.models.len(), 1, "同模型合并为一行");
        let m = &day28.models[0];
        assert_eq!(m.requests, 3, "合计 = 两源之和");
        assert_eq!(m.input_tokens, 111);
        assert_eq!(m.output_tokens, 54);
        let split = m.by_source.expect("合并口径必带 per-source 小计");
        assert_eq!(split.local.requests, 2);
        assert_eq!(split.local.input_tokens, 11);
        assert_eq!(split.router.requests, 1);
        assert_eq!(split.router.input_tokens, 100);
        assert_eq!(split.local.output_tokens, 4);
        assert_eq!(split.router.output_tokens, 50);
        // 合计与小计自洽（不重算的机械口径）。
        assert_eq!(m.requests, split.local.requests + split.router.requests);
        assert_eq!(m.input_tokens, split.local.input_tokens + split.router.input_tokens);
        assert_eq!(merged.totals.requests, 3);
        assert_eq!(merged.totals.input_tokens, 111);
        let ts = merged.totals.by_source.expect("合计同样携带小计");
        assert_eq!(ts.router.requests + ts.local.requests, merged.totals.requests);
        // 零填充桶保持零填充且模型行为空。
        assert!(merged
            .buckets
            .iter()
            .find(|b| b.bucket == "2026-09-29")
            .unwrap()
            .models
            .is_empty());
    }

    #[test]
    fn merge_carries_models_present_on_only_one_side_and_router_cause() {
        let params = day_params(1, 0);
        let local = timeseries(
            &[record("2026-09-29T01:00:00+00:00", Some("local-only"), Some(3), None)],
            params,
            now(),
        );
        let mut router = timeseries(
            &[record("2026-09-29T02:00:00+00:00", Some("router-only"), None, Some(9))],
            params,
            now(),
        );
        router.router_cause = Some("router_unreachable: boom".into());
        let merged = merge_timeseries(&local, &router);
        let models = &merged.buckets[0].models;
        assert_eq!(models.len(), 2, "单侧模型不丢");
        assert_eq!(models[0].model, "local-only");
        assert_eq!(models[0].requests, 1);
        assert_eq!(models[0].by_source.as_ref().unwrap().router.requests, 0);
        assert_eq!(models[1].model, "router-only");
        assert_eq!(models[1].by_source.as_ref().unwrap().local.requests, 0);
        assert_eq!(merged.router_cause.as_deref(), Some("router_unreachable: boom"));
    }

    // ---- wire 形状：缺省字段不上 wire（协议演进规则 1） ----

    #[test]
    fn single_source_serialization_stays_byte_identical_to_the_legacy_shape() {
        let ts = timeseries(
            &[record("2026-09-29T01:00:00+00:00", Some("m"), Some(1), Some(2))],
            day_params(1, 0),
            now(),
        );
        let v = serde_json::to_value(&ts).unwrap();
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("router_cause"), "单源响应不带 router_cause 键");
        let model = &obj["buckets"][0]["models"][0];
        assert!(
            !model.as_object().unwrap().contains_key("by_source"),
            "单源模型行不带 by_source 键"
        );
        // 字段集合与迁移前逐字一致（老客户端零破坏的机械口径）；serde_json
        // 的 Value 对象键按字典序呈现，这里按集合比较。
        let mut keys: Vec<&str> = model.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "cache_creation_tokens",
                "cache_read_tokens",
                "input_tokens",
                "model",
                "output_tokens",
                "requests",
            ]
        );
        // 老载荷（无新增字段）可被本层类型读取（合并口径的反序列化半边）。
        let legacy: Timeseries = serde_json::from_value(v).unwrap();
        assert!(legacy.router_cause.is_none());
        assert!(legacy.totals.by_source.is_none());
    }
}
