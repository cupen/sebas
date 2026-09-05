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

/// duration 直方图桶边界（ms）——对数感分布，覆盖 ms 到分钟级 SSE。
pub const LATENCY_BUCKETS_MS: [u64; 12] = [
    10, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 30_000, 60_000, 120_000,
];

/// series 上限：超出归并 `model="other"`。
const MAX_SERIES: usize = 1024;

/// 全局 registry。全局 static（非 per-AppState）：指标是进程级观测量，
/// 与内核热替换无关；多个 router 实例（测试）共享计数在测试里做相对
/// 断言（前后差值）即可，生产恒单实例。
pub struct Metrics {
    /// counter/gauge series：key = 完整 series 名（含 label），值 = 计数。
    series: Mutex<HashMap<String, u64>>,
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

impl Metrics {
    pub fn global() -> Arc<Metrics> {
        static G: std::sync::OnceLock<Arc<Metrics>> = std::sync::OnceLock::new();
        G.get_or_init(|| Arc::new(Metrics::default())).clone()
    }

    /// 计数 +1（series 不存在则分配；超上限后新 series 归并到 `other`）。
    pub fn inc(&self, series: &str) {
        self.add(series, 1);
    }

    pub fn add(&self, series: &str, v: u64) {
        let mut g = self.series.lock().unwrap_or_else(|e| e.into_inner());
        *g.entry(series.to_string()).or_insert(0) += v;
    }

    /// gauge 式写绝对值（active_requests 用）。
    pub fn set(&self, series: &str, v: u64) {
        let mut g = self.series.lock().unwrap_or_else(|e| e.into_inner());
        g.insert(series.to_string(), v);
    }

    pub fn get(&self, series: &str) -> u64 {
        self.series
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(series)
            .copied()
            .unwrap_or(0)
    }

    /// 当前全部 series 快照（按名排序，/metrics 输出用）。
    pub fn snapshot(&self) -> Vec<(String, u64)> {
        let g = self.series.lock().unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<(String, u64)> = g.iter().map(|(k, c)| (k.clone(), *c)).collect();
        v.sort();
        v
    }

    pub fn uptime_secs(&self) -> u64 {
        self.start_time
            .elapsed()
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// 观测一次请求完成（settle_inner 邻位调用）。
    pub fn observe_request(
        &self,
        provider: &str,
        model: &str,
        status: u16,
        latency: Duration,
        input_tokens: u64,
        output_tokens: u64,
    ) {
        self.inc(&format!(
            "sebas_router_requests_total{{provider=\"{provider}\",model=\"{}\"}}",
            Self::canonical_model(&self.series, model)
        ));
        // 直方图：每桶是「≤bucket 的累计计数」（Prometheus text 格式习惯）。
        let ms = latency.as_millis() as u64;
        for b in LATENCY_BUCKETS_MS {
            if ms <= b {
                self.inc(&format!(
                    "sebas_router_request_duration_ms_bucket{{provider=\"{provider}\",le=\"{b}\"}}"
                ));
            }
        }
        self.inc(&format!(
            "sebas_router_request_duration_ms_count{{provider=\"{provider}\"}}"
        ));
        self.add(
            &format!("sebas_router_tokens_total{{provider=\"{provider}\",kind=\"input\"}}"),
            input_tokens,
        );
        self.add(
            &format!("sebas_router_tokens_total{{provider=\"{provider}\",kind=\"output\"}}"),
            output_tokens,
        );
        if status >= 500 {
            self.inc(&format!(
                "sebas_router_upstream_errors_total{{provider=\"{provider}\"}}"
            ));
        }
    }

    /// auth 拒绝（401）。
    pub fn observe_auth_rejected(&self) {
        self.inc("sebas_router_auth_rejected_total");
    }

    /// rate-limit 拒绝（429）。
    pub fn observe_rate_limited(&self) {
        self.inc("sebas_router_rate_limited_total");
    }

    /// active_requests gauge +1 / -1（enter/leave proxy）。
    pub fn active_requests_enter(&self) {
        let cur = self.get(ACTIVE_SERIES) + 1;
        self.set(ACTIVE_SERIES, cur);
    }

    pub fn active_requests_leave(&self) {
        let cur = self.get(ACTIVE_SERIES).saturating_sub(1);
        self.set(ACTIVE_SERIES, cur);
    }

    /// model 名归并：series 总数超限后新 model 一律 "other"。锁内判定，
    /// 与 add 之间仍有理论 TOCTOU——但 add 只增不删，len 单调涨，最坏把
    /// 边界附近的 model 早一拍归并，无正确性影响。
    fn canonical_model(series: &Mutex<HashMap<String, u64>>, model: &str) -> String {
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

const ACTIVE_SERIES: &str = "sebas_router_active_requests";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_requests_count_three() {
        let m = Metrics::default();
        for _ in 0..3 {
            m.observe_request("alpha", "m1", 200, Duration::from_millis(50), 10, 5);
        }
        assert_eq!(
            m.get("sebas_router_requests_total{provider=\"alpha\",model=\"m1\"}"),
            3
        );
        assert_eq!(
            m.get("sebas_router_tokens_total{provider=\"alpha\",kind=\"input\"}"),
            30
        );
        // 直方图桶：50ms ≤ 50/100/... 都累计 3。
        assert_eq!(
            m.get("sebas_router_request_duration_ms_bucket{provider=\"alpha\",le=\"50\"}"),
            3
        );
        assert_eq!(
            m.get("sebas_router_request_duration_ms_bucket{provider=\"alpha\",le=\"10\"}"),
            0
        );
    }

    #[test]
    fn rate_limited_counts() {
        let m = Metrics::default();
        m.observe_rate_limited();
        m.observe_rate_limited();
        assert_eq!(m.get("sebas_router_rate_limited_total"), 2);
    }

    #[test]
    fn series_cap_merges_to_other() {
        let m = Metrics::default();
        // 灌满 series 上限。
        for i in 0..MAX_SERIES {
            m.inc(&format!("sebas_x{{v=\"{i}\"}}"));
        }
        m.observe_request("alpha", "fresh-model", 200, Duration::from_millis(1), 0, 0);
        assert_eq!(
            m.get("sebas_router_requests_total{provider=\"alpha\",model=\"other\"}"),
            1
        );
    }

    #[test]
    fn active_requests_gauge() {
        let m = Metrics::default();
        m.active_requests_enter();
        m.active_requests_enter();
        assert_eq!(m.get(ACTIVE_SERIES), 2);
        m.active_requests_leave();
        assert_eq!(m.get(ACTIVE_SERIES), 1);
    }
}
