//! Authentication, session management, CSRF protection, and rate limiting
//! for the WebUI admin dashboard.
//!
//! # Security Model
//!
//! - Password is read from `SEBAS_WEBUI_PASSWORD` env var at startup.
//! - If unset, the admin dashboard is read-only (no login required, but
//!   mutation routes return 401).
//! - Login creates a session cookie with a configurable TTL (default 24h).
//! - Every mutation route requires a valid session cookie **and** a matching
//!   `X-CSRF-Token` header (or a loopback origin as fallback for CLI tools).
//! - Login endpoint has a simple in-memory rate limiter (5 attempts / 30s).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Default session TTL: 24 hours of inactivity.
const SESSION_TTL: Duration = Duration::from_secs(86400);

/// How often a session's last-used timestamp is updated (every request).
/// This is done inline so no background reaper is needed — expired sessions
/// are pruned lazily on each auth check.
const SESSION_EXTEND_WINDOW: Duration = Duration::from_secs(300);

/// Rate limit: max attempts per IP per window.
const RATE_LIMIT_MAX: u32 = 5;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(30);

/// A single admin session.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub csrf_token: String,
    #[allow(dead_code)]
    created_at: Instant,
    last_used: Instant,
}

impl Session {
    fn new() -> Self {
        let id = generate_token(32);
        let csrf_token = generate_token(32);
        Self {
            id,
            csrf_token,
            created_at: Instant::now(),
            last_used: Instant::now(),
        }
    }

    /// Check if the session has expired due to inactivity.
    fn is_expired(&self) -> bool {
        self.last_used.elapsed() > SESSION_TTL
    }

    /// Update the last-used timestamp (amortized, not on every request).
    fn touch(&mut self) {
        self.last_used = Instant::now();
    }
}

/// Thread-safe session store.
#[derive(Debug, Clone)]
pub struct SessionStore {
    inner: Arc<Mutex<SessionStoreInner>>,
}

#[derive(Debug)]
struct SessionStoreInner {
    sessions: HashMap<String, Session>,
    /// Rate-limit state: IP → (attempts, window_start)
    rate_limit: HashMap<String, (u32, Instant)>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SessionStoreInner {
                sessions: HashMap::new(),
                rate_limit: HashMap::new(),
            })),
        }
    }

    /// Create a new session, returning its ID and CSRF token.
    pub async fn create(&self) -> (String, String) {
        let mut inner = self.inner.lock().await;
        let session = Session::new();
        let id = session.id.clone();
        let csrf = session.csrf_token.clone();
        inner.sessions.insert(id.clone(), session);
        (id, csrf)
    }

    /// Validate a session cookie value. Returns the CSRF token if valid, or
    /// an error message. Lazily prunes expired sessions.
    pub async fn validate(&self, session_id: &str) -> Result<String, &'static str> {
        let mut inner = self.inner.lock().await;

        // Lazy pruning: remove expired sessions
        inner.sessions.retain(|_, s| !s.is_expired());

        match inner.sessions.get_mut(session_id) {
            Some(session) => {
                if session.is_expired() {
                    inner.sessions.remove(session_id);
                    Err("session expired")
                } else {
                    // Extend the session's lifetime on activity
                    if session.last_used.elapsed() > SESSION_EXTEND_WINDOW {
                        session.touch();
                    }
                    Ok(session.csrf_token.clone())
                }
            }
            None => Err("invalid session"),
        }
    }

    /// Remove a session (logout).
    pub async fn remove(&self, session_id: &str) {
        self.inner.lock().await.sessions.remove(session_id);
    }

    /// Check rate limit for an IP address. Returns true if the request is
    /// allowed, false if rate-limited.
    pub async fn check_rate_limit(&self, ip: &str) -> bool {
        let mut inner = self.inner.lock().await;
        let now = Instant::now();

        let entry = inner.rate_limit.entry(ip.to_string()).or_insert((0, now));
        if entry.1.elapsed() > RATE_LIMIT_WINDOW {
            // Window expired, reset
            *entry = (1, now);
            true
        } else if entry.0 >= RATE_LIMIT_MAX {
            false
        } else {
            entry.0 += 1;
            true
        }
    }

    /// Reset rate limit for an IP (on successful login).
    pub async fn reset_rate_limit(&self, ip: &str) {
        self.inner.lock().await.rate_limit.remove(ip);
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate a random hex token of the given byte length (result is 2×len hex chars).
/// 会话 token 属安全凭据，必须来自 OS CSPRNG（旧实现是时间种子 LCG，可预测）。
pub fn generate_token(byte_len: usize) -> String {
    hex::encode(crate::auth::random_bytes(byte_len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_created_and_validated() {
        let store = SessionStore::new();
        let (id, csrf) = store.create().await;
        assert!(!id.is_empty());
        assert!(!csrf.is_empty());

        let result = store.validate(&id).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), csrf);
    }

    #[tokio::test]
    async fn invalid_session_rejected() {
        let store = SessionStore::new();
        let result = store.validate("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn removed_session_rejected() {
        let store = SessionStore::new();
        let (id, _) = store.create().await;
        store.remove(&id).await;
        let result = store.validate(&id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rate_limit_blocks_excessive_attempts() {
        let store = SessionStore::new();
        let ip = "127.0.0.1";

        // First 5 attempts should succeed
        for _ in 0..5 {
            assert!(store.check_rate_limit(ip).await);
        }

        // 6th attempt should be blocked
        assert!(!store.check_rate_limit(ip).await);

        // Reset should allow again
        store.reset_rate_limit(ip).await;
        assert!(store.check_rate_limit(ip).await);
    }

    #[tokio::test]
    async fn rate_limit_per_ip_is_independent() {
        let store = SessionStore::new();

        for _ in 0..5 {
            assert!(store.check_rate_limit("127.0.0.1").await);
        }
        assert!(!store.check_rate_limit("127.0.0.1").await);

        // Different IP is not affected
        assert!(store.check_rate_limit("10.0.0.1").await);
    }
}
