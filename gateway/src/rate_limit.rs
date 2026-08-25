//! 基于 token 的限流中间件（sebas-lva P0）。
//!
//! 按鉴权后的下游 client/token（而非 IP）限流——gateway 常用于受限 token 卖给
//! 多客户端、单 IP 的售卖场景，IP 维度无法区分合法共享与滥用。key 从请求
//! `Authorization: Bearer` / `x-api-key` 提取（复用 `crate::auth::extract_key`），
//! 与鉴权同一把 key。
//!
//! 算法采用 **token-bucket**（令牌桶）：
//! - 容量 `capacity`：允许的瞬时突发请求数（桶深）；
//! - 补充速率 `refill_per_sec`：每秒归还的令牌数（长期平均速率）。
//!   相比固定窗口（fixed window），token-bucket 在窗口边界不出现「整窗放行
//!   后瞬间全拒」的毛刺，更适合对齐上游请求速率平滑的真实流量；且实现同样
//!   简单（惰性补充 + 计数器），无需定时器。
//!
//! 实现：`RateLimiter` 是共享状态（`Arc<Mutex<...>>`），内部对每个 token/缺省
//! key 维护一个 `Bucket`。`try_acquire` 惰性按耗时补充令牌，不够则拒绝。
//! 缺省（不限流）时中间件直接放行，`disabled` 下无锁开销。
//!
//! 超限返回 429 Too Many Requests（协议面由 `resolve_target` 嗅探，与 auth.rs
//! 同款兜底为 OpenAi）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

use crate::auth::extract_key;
use crate::config::RateLimitConfig;
use crate::error::error_response;
use crate::proto::{WireProtocol, resolve_target};
use crate::server::AppState;

/// 单 token 的令牌桶。`tokens` 是「已累积可用令牌」的浮点计数。容量/补充速率
/// 按桶构建；起始满桶（允许 `capacity` 个请求立即通过作为突发）。
#[derive(Clone)]
struct Bucket {
    capacity: f64,
    refill_per_sec: f64,
    tokens: f64,
    last: Instant,
}

impl Bucket {
    fn new(capacity: f64, refill_per_sec: f64) -> Self {
        Bucket {
            capacity,
            refill_per_sec,
            tokens: capacity,
            last: Instant::now(),
        }
    }

    /// 尝试消耗 1 个令牌。惰性补充：先按 `now - last` 补足（封顶到容量），再
    /// 决定是否放行。用 `f64` 计数避免每个请求借位取整的微漂。
    fn try_acquire(&mut self, now: Instant) -> bool {
        if self.capacity <= 0.0 {
            return false;
        }
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// 共享限流状态：token → 桶。挂在 `AppState`（`Arc<Mutex>` 提供
/// `Clone + Send + Sync + 'static`）。
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<String, Bucket>>>,
    /// 是否启用限流。禁用时 `try_acquire` 恒真，零锁开销。
    enabled: bool,
}

impl RateLimiter {
    /// 由配置构建。`bucket_params` 为 None（不限流）→ `enabled=false`，中间件
    /// 直接放行。预置一个 `"*"` 缺省桶：对无 key 请求（未配置 auth_token 的裸奔
    /// 场景）做网关整体限流，防止单进程被打爆。缺失 key 的桶按 `"*"` 参数克隆。
    pub fn from_config(cfg: &RateLimitConfig) -> Self {
        match cfg.bucket_params() {
            Some((cap, refill)) => {
                let cap = cap as f64;
                let mut buckets = HashMap::new();
                buckets.insert("*".to_string(), Bucket::new(cap, refill));
                RateLimiter {
                    inner: Arc::new(Mutex::new(buckets)),
                    enabled: true,
                }
            }
            None => RateLimiter {
                inner: Arc::new(Mutex::new(HashMap::new())),
                enabled: false,
            },
        }
    }

    /// 尝试获取一个令牌。返回是否放行。
    pub fn try_acquire(&self, key: &str) -> bool {
        if !self.enabled {
            return true;
        }
        let mut buckets = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        if let Some(b) = buckets.get_mut(key) {
            b.try_acquire(now)
        } else {
            // 首次见到该 key：按缺省桶参数克隆新桶。
            let proto = buckets.get("*").expect("default bucket seeded").clone();
            buckets.insert(key.to_string(), proto);
            buckets
                .get_mut(key)
                .expect("just inserted")
                .try_acquire(now)
        }
    }
}

/// 渲染 429。协议面由 `resolve_target` 嗅探。message 通用，不含 key/限额。
fn too_many(headers: &axum::http::HeaderMap, path: &str) -> Response {
    let proto = resolve_target(headers, path)
        .map(|t| t.protocol)
        .unwrap_or(WireProtocol::OpenAi);
    error_response(
        proto,
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limit_error",
        "rate limit exceeded",
    )
}

/// 限流中间件。挂在鉴权（`require_key`）**之后**、`proxy::handle` 之前，只对
/// 放行的合法请求计数——「鉴权后的 client/token」维度。`/healthz` 豁免。
pub async fn rate_limit(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // 缺省（未配置限流）或 debug 模式下直接放行，零锁开销。
    if state.cfg.debug || !state.rate_limiter.enabled {
        return next.run(req).await;
    }
    // /healthz 豁免——健康探测不计费、不占令牌。
    if req.uri().path() == "/healthz" {
        return next.run(req).await;
    }
    // 按鉴权 token 维度限流；无 key（裸奔场景）落到缺省桶。
    let key = extract_key(req.headers()).unwrap_or_else(|| "anonymous".to_string());
    if state.rate_limiter.try_acquire(&key) {
        next.run(req).await
    } else {
        too_many(req.headers(), req.uri().path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(capacity: u64, refill_per_sec: f64) -> RateLimitConfig {
        RateLimitConfig {
            rpm: None,
            capacity: Some(capacity),
            refill_per_sec,
        }
    }

    #[test]
    fn disabled_always_acquires() {
        let rl = RateLimiter::from_config(&RateLimitConfig::default());
        assert!(!rl.enabled);
        for _ in 0..1000 {
            assert!(rl.try_acquire("any"));
        }
    }

    #[test]
    fn capacity_burst_allows_capacity_then_rejects() {
        let rl = RateLimiter::from_config(&cfg(3, 1.0));
        // 满桶 3 个令牌：前 3 次全过
        for _ in 0..3 {
            assert!(rl.try_acquire("sk-a"), "within capacity must pass");
        }
        // 第 4 次：桶空
        assert!(!rl.try_acquire("sk-a"), "beyond capacity must be rejected");
    }

    #[test]
    fn different_keys_have_independent_buckets() {
        let rl = RateLimiter::from_config(&cfg(1, 1.0));
        assert!(rl.try_acquire("sk-a"));
        assert!(!rl.try_acquire("sk-a"), "sk-a bucket empty");
        // sk-b 独立桶，仍可过
        assert!(rl.try_acquire("sk-b"), "sk-b has its own bucket");
    }

    #[test]
    fn refill_restores_token_over_time() {
        let rl = RateLimiter::from_config(&cfg(1, 60.0)); // 1 秒补满 1 令牌
        assert!(rl.try_acquire("sk-a"));
        assert!(!rl.try_acquire("sk-a"), "桶空");
        // 等补满后立即再取（> 1/60s 已补满 1 令牌）。
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(
            rl.try_acquire("sk-a"),
            "refill over time must restore a token"
        );
    }

    #[test]
    fn bucket_try_acquire_lazy_refill() {
        let mut b = Bucket::new(1.0, 0.01); // 100s 补 1 令牌
        assert!(b.try_acquire(Instant::now()), "full bucket passes");
        assert!(!b.try_acquire(Instant::now()), "empty bucket rejects");
    }
}
