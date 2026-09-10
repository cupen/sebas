//! 手写 Prometheus 指标（Task 5.1，design D6：不引 prometheus crate）。
//!
//! Registry = `HashMap<series_key, AtomicU64>` + `DashMap` 式并发（这里用
//! std Mutex 包 HashMap；计数是纳秒级 fetch_add）。series 上限 1024，超出
//! 归并到 `model="other"`（防 label 基数爆炸）。
//!
//! 观测点：
//! - `settle_inner` 邻位（proxy 完成路径）：requests_total / duration 直方图
//!   桶（ms）/ tokens / upstream_errors；
//! - auth 拒绝（401）、rate-limit 拒绝（429）：auth_rejected / rate_limited；
//! - active_requests：进入 proxy 时 +1、settle 时 -1；
//! - start_time：进程启动时刻（/metrics 输出 uptime 基准）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// duration 直方图桶边界（秒）——覆盖 10ms 到 2 分钟级 SSE。
pub const LATENCY_BUCKETS_S: [f64; 12] = [
    0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
];

/// series 上限：超出归并 `model="other"`。
const MAX_SERIES: usize = 1024;

/// 全局 registry。全局 static（非 per-AppState）：指标是进程级观测量，
/// 与内核热替换无关；多个 router 实例（测试）共享计数在测试里做相对
/// 断言（前后差值）即可，生产恒单实例。
pub struct Metrics {
    /// counter/gauge series：key = 完整 series 名（含 label），值 = 计数。
    series: Mutex<HashMap<String, f64>>,
    /// 进程启动时刻（uptime 基准）。
    start_time: SystemTime,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics {
            series: Mutex::new(HashMap::new()),
            start_time: SystemTime::now(),
        }
    }
}

/// 一次完成请求的观测（settle_inner 汇点 → metrics）。字段对应 spec labels。
pub struct RequestObservation<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub protocol: &'a str,
    pub status: u16,
    pub latency: Duration,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
}

impl Metrics {
    pub fn global() -> Arc<Metrics> {
        static G: std::sync::OnceLock<Arc<Metrics>> = std::sync::OnceLock::new();
        G.get_or_init(|| Arc::new(Metrics::default())).clone()
    }

    /// 计数 +1（series 不存在则分配；超上限后新 series 归并到 `other`）。
    pub fn inc(&self, series: &str) {
        self.add_f64(series, 1.0);
    }

    pub fn add(&self, series: &str, v: u64) {
        self.add_f64(series, v as f64);
    }

    pub fn add_f64(&self, series: &str, v: f64) {
        let mut g = self.series.lock().unwrap_or_else(|e| e.into_inner());
        *g.entry(series.to_string()).or_insert(0.0) += v;
    }

    /// gauge 式写绝对值（active_requests 用）。
    pub fn set(&self, series: &str, v: f64) {
        let mut g = self.series.lock().unwrap_or_else(|e| e.into_inner());
        g.insert(series.to_string(), v);
    }

    pub fn get(&self, series: &str) -> f64 {
        self.series
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(series)
            .copied()
            .unwrap_or(0.0)
    }

    /// 当前全部 series 快照（按名排序，/metrics 输出用）。
    pub fn snapshot(&self) -> Vec<(String, f64)> {
        let g = self.series.lock().unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<(String, f64)> = g.iter().map(|(k, c)| (k.clone(), *c)).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// 进程启动时刻（unix 秒），router_start_time_seconds 数据源。
    pub fn start_time_unix(&self) -> u64 {
        self.start_time
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    pub fn uptime_secs(&self) -> u64 {
        self.start_time
            .elapsed()
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// 观测一次请求完成（settle_inner 邻位调用）。router-metrics spec「Prometheus
    /// exposition」：family 名不带 `sebas_` 前缀（`router_*`），labels 全量。
    pub fn observe_request(&self, obs: RequestObservation<'_>) {
        let model = Self::canonical_model(&self.series, obs.model);
        let proto = obs.protocol;
        // requests_total{provider,model,protocol,status}
        self.inc(&format!(
            "router_requests_total{{provider=\"{}\",model=\"{model}\",protocol=\"{proto}\",status=\"{}\"}}",
            obs.provider, obs.status
        ));
        // 直方图（秒）{provider,model,protocol}：每桶累计 ≤bucket，外加 +Inf 与 _sum/_count。
        let secs = obs.latency.as_secs_f64();
        for b in LATENCY_BUCKETS_S {
            if secs <= b {
                self.inc(&format!(
                    "router_request_duration_seconds_bucket{{provider=\"{}\",model=\"{model}\",protocol=\"{proto}\",le=\"{b}\"}}",
                    obs.provider
                ));
            }
        }
        self.inc(&format!(
            "router_request_duration_seconds_bucket{{provider=\"{}\",model=\"{model}\",protocol=\"{proto}\",le=\"+Inf\"}}",
            obs.provider
        ));
        self.add_f64(
            &format!(
                "router_request_duration_seconds_sum{{provider=\"{}\",model=\"{model}\",protocol=\"{proto}\"}}",
                obs.provider
            ),
            secs,
        );
        self.inc(&format!(
            "router_request_duration_seconds_count{{provider=\"{}\",model=\"{model}\",protocol=\"{proto}\"}}",
            obs.provider
        ));
        // tokens_total{provider,type in input|output|cache_read|cache_creation}
        self.add(
            &format!("router_tokens_total{{provider=\"{}\",type=\"input\"}}", obs.provider),
            obs.input_tokens,
        );
        self.add(
            &format!("router_tokens_total{{provider=\"{}\",type=\"output\"}}", obs.provider),
            obs.output_tokens,
        );
        if let Some(c) = obs.cache_read_tokens {
            self.add(
                &format!("router_tokens_total{{provider=\"{}\",type=\"cache_read\"}}", obs.provider),
                c,
            );
        }
        if let Some(c) = obs.cache_creation_tokens {
            self.add(
                &format!("router_tokens_total{{provider=\"{}\",type=\"cache_creation\"}}", obs.provider),
                c,
            );
        }
        if obs.status >= 500 {
            self.inc(&format!(
                "router_upstream_errors_total{{provider=\"{}\"}}",
                obs.provider
            ));
        }
    }

    /// auth 拒绝（401）。
    pub fn observe_auth_rejected(&self) {
        self.inc("router_auth_rejected_total");
    }

    /// rate-limit 拒绝（429）。
    pub fn observe_rate_limited(&self) {
        self.inc("router_rate_limited_total");
    }

    /// active_requests gauge +1 / -1（enter/leave proxy）。
    pub fn active_requests_enter(&self) {
        let cur = self.get(ACTIVE_SERIES) + 1.0;
        self.set(ACTIVE_SERIES, cur);
    }

    pub fn active_requests_leave(&self) {
        let cur = (self.get(ACTIVE_SERIES) - 1.0).max(0.0);
        self.set(ACTIVE_SERIES, cur);
    }

    /// model 名归并：series 总数超限后新 model 一律 "other"。锁内判定，
    /// 与 add 之间仍有理论 TOCTOU——但 add 只增不删，len 单调涨，最坏把
    /// 边界附近的 model 早一拍归并，无正确性影响。
    fn canonical_model(series: &Mutex<HashMap<String, f64>>, model: &str) -> String {
        let distinct = series.lock().unwrap_or_else(|e| e.into_inner()).len();
        if distinct >= MAX_SERIES {
            "other".to_string()
        } else if model.is_empty() {
            "unknown".to_string()
        } else {
            model.to_string()
        }
    }
}

const ACTIVE_SERIES: &str = "router_active_requests";

#[cfg(test)]
mod tests {
    use super::*;

    fn obs<'a>(provider: &'a str, model: &'a str, status: u16, secs: f64, in_tok: u64, out_tok: u64) -> RequestObservation<'a> {
        RequestObservation {
            provider,
            model,
            protocol: "openai",
            status,
            latency: Duration::from_secs_f64(secs),
            input_tokens: in_tok,
            output_tokens: out_tok,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        }
    }

    #[test]
    fn three_requests_count_three() {
        let m = Metrics::default();
        for _ in 0..3 {
            m.observe_request(obs("alpha", "m1", 200, 0.05, 10, 5));
        }
        assert_eq!(
            m.get("router_requests_total{provider=\"alpha\",model=\"m1\",protocol=\"openai\",status=\"200\"}"),
            3.0
        );
        assert_eq!(
            m.get("router_tokens_total{provider=\"alpha\",type=\"input\"}"),
            30.0
        );
        // 直方图桶（秒）：0.05s ≤ 0.05/0.1/... 都累计 3；≤0.01 为 0。
        assert_eq!(
            m.get("router_request_duration_seconds_bucket{provider=\"alpha\",model=\"m1\",protocol=\"openai\",le=\"0.05\"}"),
            3.0
        );
        assert_eq!(
            m.get("router_request_duration_seconds_bucket{provider=\"alpha\",model=\"m1\",protocol=\"openai\",le=\"0.01\"}"),
            0.0
        );
        // +Inf 与 sum/count 都累计。
        assert_eq!(
            m.get("router_request_duration_seconds_count{provider=\"alpha\",model=\"m1\",protocol=\"openai\"}"),
            3.0
        );
    }

    #[test]
    fn cache_tokens_recorded() {
        let m = Metrics::default();
        let mut o = obs("alpha", "m1", 200, 0.05, 10, 5);
        o.cache_read_tokens = Some(7);
        o.cache_creation_tokens = Some(3);
        m.observe_request(o);
        assert_eq!(m.get("router_tokens_total{provider=\"alpha\",type=\"cache_read\"}"), 7.0);
        assert_eq!(m.get("router_tokens_total{provider=\"alpha\",type=\"cache_creation\"}"), 3.0);
    }

    #[test]
    fn rate_limited_counts() {
        let m = Metrics::default();
        m.observe_rate_limited();
        m.observe_rate_limited();
        assert_eq!(m.get("router_rate_limited_total"), 2.0);
    }

    #[test]
    fn start_time_is_unix_seconds() {
        let m = Metrics::default();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(m.start_time_unix() <= now && m.start_time_unix() > 0);
    }

    #[test]
    fn series_cap_merges_to_other() {
        let m = Metrics::default();
        // 灌满 series 上限。
        for i in 0..MAX_SERIES {
            m.inc(&format!("sebas_x{{v=\"{i}\"}}"));
        }
        m.observe_request(obs("alpha", "fresh-model", 200, 0.001, 0, 0));
        assert_eq!(
            m.get("router_requests_total{provider=\"alpha\",model=\"other\",protocol=\"openai\",status=\"200\"}"),
            1.0
        );
    }

    #[test]
    fn active_requests_gauge() {
        let m = Metrics::default();
        m.active_requests_enter();
        m.active_requests_enter();
        assert_eq!(m.get(ACTIVE_SERIES), 2.0);
        m.active_requests_leave();
        assert_eq!(m.get(ACTIVE_SERIES), 1.0);
    }
}
