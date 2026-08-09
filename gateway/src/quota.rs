//! per-key 限流与配额记账（Task 6，spec §4.5）。
//!
//! `Quota` 持 `Mutex<HashMap<String, KeyUsage>>`（map key = 下游 key 字符串）。
//! `check` 依次判定日 token 配额与 RPM 分钟窗口：日配额已超 → Deny（到午夜
//! 秒数），RPM 窗口满 → Deny（窗口剩余秒数），否则 RPM 计数 +1 放行。
//! `record_tokens` 由 usage 路径在响应结算后调用，本请求消耗在下次 check 才
//! 生效（设计文档认可的近似语义）。窗口/日期翻转逻辑集中在 check/record_tokens
//! 内。
//!
//! Task 7 在 proxy 调 `check` 渲染 429；Task 8 在 usage 路径调 `record_tokens`。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use chrono::{Local, NaiveDate, Timelike};

use crate::auth::KeyIdentity;

/// 单 key 的实时用量状态。RPM 用分钟固定窗口（`Instant`），日配额用
/// `chrono::Local::now().date_naive()`，跨天自动清零。同模块测试可直接操作
/// 字段构造时间态（spec §4.5 认可的近似语义）。
#[derive(Debug, Clone)]
pub struct KeyUsage {
    /// 分钟窗口起点。窗口 [window_start, window_start + 60s) 内的请求数计
    /// `rpm_count`；窗口过期由 `check`/`record_tokens` 检测并重置。
    pub window_start: Instant,
    /// 当前分钟窗口内已计数请求数。
    pub rpm_count: u32,
    /// 当天累计 token 数（事后记账：响应结算后由 `record_tokens` 补记）。
    pub daily_tokens: u64,
    /// 当天日期（用于跨天清零判定）。
    pub day: NaiveDate,
}

impl KeyUsage {
    /// 构造一个起点为 `now`、日期为 `today` 的空白用量。
    fn fresh(now: Instant, today: NaiveDate) -> Self {
        KeyUsage {
            window_start: now,
            rpm_count: 0,
            daily_tokens: 0,
            day: today,
        }
    }
}

/// 限流/配额判定结果。`Deny` 由 Task 7 proxy 渲染成 429 + `Retry-After`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaVerdict {
    /// 放行（已对 RPM 计数 +1）。
    Allow,
    /// 拒绝。`retry_after_secs` 给 `Retry-After` header；`reason` 为机器可读原因
    /// 串，供 proxy 按 protocol 映射成对应错误类型。
    Deny {
        retry_after_secs: u64,
        reason: &'static str,
    },
}

/// 日 token 配额超限的 `reason` 串。
pub const REASON_DAILY_TOKEN_QUOTA: &str = "daily_token_quota_exceeded";
/// RPM 分钟窗口超限的 `reason` 串。
pub const REASON_RPM: &str = "rpm_exceeded";

/// RPM 分钟固定窗口长度（秒）。
const RPM_WINDOW_SECS: u64 = 60;
/// 一天秒数（86400），用于推算到本地午夜的剩余秒数。
const SECS_PER_DAY: u64 = 86_400;

/// per-key 令牌桶 + 配额记账。map key = 下游 key 字符串。`Default` 构造空 map。
#[derive(Debug, Default)]
pub struct Quota {
    inner: Mutex<HashMap<String, KeyUsage>>,
}

impl Quota {
    pub fn new() -> Self {
        Self::default()
    }

    /// 限流/配额判定。日 token 已超 → Deny（到午夜秒数）；RPM 分钟窗口满 →
    /// Deny（窗口剩余秒数）；否则 RPM 计数 +1 放行。
    ///
    /// token 是事后记账：本请求消耗在响应结算后由 `record_tokens` 补记，
    /// 下一次 check 才生效。
    pub fn check(&self, key: &KeyIdentity) -> QuotaVerdict {
        let now = Instant::now();
        let today = Local::now().date_naive();
        let rpm_limit = key.config.rpm;
        let daily_limit = key.config.daily_token_quota;

        let mut inner = self.inner.lock().expect("quota mutex poisoned");
        let entry = inner
            .entry(key.config.key.clone())
            .or_insert_with(|| KeyUsage::fresh(now, today));

        // 跨天清零：日配额按日翻转，先于配额判定。
        if entry.day != today {
            entry.day = today;
            entry.daily_tokens = 0;
        }

        // 日 token 配额判定（事后记账：已记 token 超过配额 → Deny 到午夜）。
        if let Some(limit) = daily_limit
            && entry.daily_tokens >= limit
        {
            return QuotaVerdict::Deny {
                retry_after_secs: seconds_to_midnight(),
                reason: REASON_DAILY_TOKEN_QUOTA,
            };
        }

        // RPM 分钟窗口判定（无 RPM 限制则跳过）。
        if let Some(rpm) = rpm_limit {
            let elapsed = now.duration_since(entry.window_start);
            // 窗口过期则重置，开启新窗口；此时新窗口已用 0 秒。
            let window_elapsed_secs = if elapsed.as_secs() >= RPM_WINDOW_SECS {
                entry.window_start = now;
                entry.rpm_count = 0;
                0
            } else {
                elapsed.as_secs()
            };
            if entry.rpm_count >= rpm {
                // 窗口内剩余秒数。window_elapsed_secs ∈ [0,59]，故 remaining ∈ [1,60]。
                let remaining = RPM_WINDOW_SECS - window_elapsed_secs;
                return QuotaVerdict::Deny {
                    retry_after_secs: remaining,
                    reason: REASON_RPM,
                };
            }
            entry.rpm_count += 1;
        }

        QuotaVerdict::Allow
    }

    /// 响应结算后由 usage 路径调用，补记本次请求消耗的 token。下次 `check`
    /// 才纳入日配额判定（事后记账）。跨天时先对齐日期并清零。
    pub fn record_tokens(&self, key: &str, tokens: u64) {
        let now = Instant::now();
        let today = Local::now().date_naive();
        let mut inner = self.inner.lock().expect("quota mutex poisoned");
        let entry = inner
            .entry(key.to_string())
            .or_insert_with(|| KeyUsage::fresh(now, today));
        // 跨天清零：写入前对齐日期，避免昨日 token 累到今天。
        if entry.day != today {
            entry.day = today;
            entry.daily_tokens = 0;
        }
        entry.daily_tokens = entry.daily_tokens.saturating_add(tokens);
    }
}

/// 计算到本地午夜的剩余秒数（用于日配额超限时的 `Retry-After`）。
/// 至少返回 1，避免 0 落到 `Retry-After` header。
fn seconds_to_midnight() -> u64 {
    let now = Local::now();
    let secs_since_midnight = now.num_seconds_from_midnight();
    SECS_PER_DAY
        .saturating_sub(secs_since_midnight as u64)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::KeyConfig;
    use std::time::Duration;

    /// 构造一个 `KeyIdentity`（仅填限流/配额字段，其余置空）。
    fn identity(key: &str, rpm: Option<u32>, daily_token_quota: Option<u64>) -> KeyIdentity {
        KeyIdentity {
            config: KeyConfig {
                key: key.into(),
                key_env: None,
                name: String::new(),
                rpm,
                daily_token_quota,
                allow_models: vec![],
                default_provider: None,
            },
        }
    }

    // -------------------- 1. RPM 到限 Deny 且 retry_after∈(0,60] --------------------

    #[test]
    fn rpm_at_limit_denies_with_retry_after_in_window() {
        let q = Quota::new();
        let key = identity("sk-rpm", Some(3), None);
        // 3 次放行
        for i in 0..3 {
            assert_eq!(
                q.check(&key),
                QuotaVerdict::Allow,
                "call #{i} should be Allow"
            );
        }
        // 第 4 次 Deny（RPM 窗口满）
        match q.check(&key) {
            QuotaVerdict::Deny {
                retry_after_secs,
                reason,
            } => {
                assert!(
                    retry_after_secs > 0 && retry_after_secs <= 60,
                    "retry_after should be in (0,60]: got {retry_after_secs}"
                );
                assert_eq!(reason, REASON_RPM);
            }
            QuotaVerdict::Allow => panic!("expected Deny at rpm limit"),
        }
    }

    // -------- 2. 拨回 window_start 模拟窗口重置放行 --------

    #[test]
    fn rpm_window_reset_allows_after_rollback() {
        let q = Quota::new();
        let key = identity("sk-reset", Some(2), None);
        assert_eq!(q.check(&key), QuotaVerdict::Allow);
        assert_eq!(q.check(&key), QuotaVerdict::Allow);
        assert!(matches!(q.check(&key), QuotaVerdict::Deny { .. }));

        // 拨回 window_start 到 120s 前 → 窗口已过期 → 下次 check 重置放行。
        {
            let mut inner = q.inner.lock().expect("mutex");
            let entry = inner.get_mut("sk-reset").expect("entry present");
            entry.window_start = Instant::now() - Duration::from_secs(120);
        }
        assert_eq!(q.check(&key), QuotaVerdict::Allow);
    }

    // -------- 3. record_tokens 到顶后 Deny --------

    #[test]
    fn daily_token_quota_record_then_deny() {
        let q = Quota::new();
        let key = identity("sk-daily", None, Some(1000));
        // 记账前放行
        assert_eq!(q.check(&key), QuotaVerdict::Allow);
        // 记账到顶
        q.record_tokens("sk-daily", 1000);
        // 现在 Deny（日配额超限）
        match q.check(&key) {
            QuotaVerdict::Deny {
                retry_after_secs,
                reason,
            } => {
                assert!(
                    retry_after_secs > 0,
                    "retry_after to midnight should be positive: {retry_after_secs}"
                );
                assert_eq!(reason, REASON_DAILY_TOKEN_QUOTA);
            }
            QuotaVerdict::Allow => panic!("expected Deny after daily quota filled"),
        }
    }

    // -------- 4. 拨回 day 模拟跨天重置 --------

    #[test]
    fn daily_quota_resets_on_day_rollback() {
        let q = Quota::new();
        let key = identity("sk-day", None, Some(1000));
        q.record_tokens("sk-day", 1000);
        assert!(matches!(q.check(&key), QuotaVerdict::Deny { .. }));

        // 拨回 day 到昨天 → 跨天 → 下次 check 清零放行。
        {
            let mut inner = q.inner.lock().expect("mutex");
            let entry = inner.get_mut("sk-day").expect("entry present");
            entry.day = Local::now().date_naive().pred_opt().unwrap();
        }
        assert_eq!(q.check(&key), QuotaVerdict::Allow);
    }

    // -------- 5. 无限制 key 永放行 --------

    #[test]
    fn unrestricted_key_always_allows() {
        let q = Quota::new();
        let key = identity("sk-unlimited", None, None);
        for _ in 0..100 {
            assert_eq!(q.check(&key), QuotaVerdict::Allow);
        }
        // 记大额 token 仍放行（无日配额限制）
        q.record_tokens("sk-unlimited", 1_000_000_000);
        assert_eq!(q.check(&key), QuotaVerdict::Allow);
    }

    // -------- 6. rpm=0 退配：恒 Deny 且 retry_after ∈ (0,60]（窗口过期后仍 >0） --------

    #[test]
    fn rpm_zero_always_denies_with_positive_retry_after() {
        let q = Quota::new();
        let key = identity("sk-zero", Some(0), None);
        // 即便窗口刚起也立刻 Deny
        match q.check(&key) {
            QuotaVerdict::Deny {
                retry_after_secs,
                reason,
            } => {
                assert!(
                    retry_after_secs > 0 && retry_after_secs <= 60,
                    "retry_after in (0,60]: got {retry_after_secs}"
                );
                assert_eq!(reason, REASON_RPM);
            }
            QuotaVerdict::Allow => panic!("rpm=0 should deny"),
        }
        // 拨回 window_start 模拟窗口过期，再 check：仍 Deny 且 retry_after > 0
        // （验证重置后 remaining 不退化为 0）。
        {
            let mut inner = q.inner.lock().expect("mutex");
            let entry = inner.get_mut("sk-zero").expect("entry present");
            entry.window_start = Instant::now() - Duration::from_secs(120);
        }
        match q.check(&key) {
            QuotaVerdict::Deny {
                retry_after_secs, ..
            } => {
                assert!(
                    retry_after_secs > 0 && retry_after_secs <= 60,
                    "retry_after after window reset in (0,60]: got {retry_after_secs}"
                );
            }
            QuotaVerdict::Allow => panic!("rpm=0 should deny even after reset"),
        }
    }
}
