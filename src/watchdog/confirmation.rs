//! Confirmation grants for the watchdog control plane.
//!
//! This module provides a single-use confirmation grant system bound to
//! a specific principal, action, channel, instance, and optional action
//! parameters with expiry.
//!
//! # Design
//!
//! A `ConfirmationGrant` is created by `ConfirmationService::create_grant` and
//! redeemed by `ConfirmationService::redeem`. The grant is bound to the
//! principal (who), action (what), channel (where), instance (which), and
//! optional action parameters. Once redeemed, the grant cannot be used again.
//!
//! All state is kept in-memory; no durable storage is used. The service is
//! thread-safe and guarantees that concurrent redeems for the same token
//! result in exactly one success.

use crate::watchdog::auth::AssertionPrincipal;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// ─── Grant Model ───────────────────────────────────────────────────────────────

/// A single-use confirmation grant bound to a specific principal, action, channel,
/// instance, and optional action parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationGrant {
    /// Unique token identifier.
    pub token: String,
    /// The principal who may redeem this grant.
    pub principal: AssertionPrincipal,
    /// The action being authorized (e.g. "restart", "update").
    pub action: String,
    /// The channel through which redemption is valid (e.g. chat_id, session_id).
    pub channel: String,
    /// The specific instance the action applies to (e.g. operation_id, service name).
    pub instance: String,
    /// Optional action parameters that must match on redemption.
    pub params: HashMap<String, String>,
    /// Unix timestamp (seconds) after which the grant expires.
    pub expires_at: u64,
}

/// Internal state of a grant.
#[derive(Debug, Clone)]
struct GrantState {
    grant: ConfirmationGrant,
    redeemed: bool,
}

// ─── Error Types ───────────────────────────────────────────────────────────────

/// Why a confirmation grant was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmationError {
    /// The token does not correspond to any known grant.
    NotFound,
    /// The grant has already been redeemed.
    AlreadyRedeemed,
    /// The grant has expired.
    Expired,
    /// The redeeming principal does not match the grant's principal.
    PrincipalMismatch,
    /// The redeeming channel does not match the grant's channel.
    ChannelMismatch,
    /// An action parameter does not match the grant's parameter.
    ParamsMismatch {
        field: String,
        expected: String,
        got: String,
    },
}

impl std::fmt::Display for ConfirmationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfirmationError::NotFound => write!(f, "grant not found"),
            ConfirmationError::AlreadyRedeemed => write!(f, "grant already redeemed"),
            ConfirmationError::Expired => write!(f, "grant has expired"),
            ConfirmationError::PrincipalMismatch => write!(f, "principal mismatch"),
            ConfirmationError::ChannelMismatch => write!(f, "channel mismatch"),
            ConfirmationError::ParamsMismatch {
                field,
                expected,
                got,
            } => write!(
                f,
                "parameter mismatch: {field} expected \"{expected}\", got \"{got}\""
            ),
        }
    }
}

// ─── ConfirmationService ───────────────────────────────────────────────────────

/// Service for creating and redeeming single-use confirmation grants.
///
/// All state is kept in-memory. The service is thread-safe and ensures that
/// concurrent redeems for the same token result in exactly one success.
pub struct ConfirmationService {
    grants: Mutex<HashMap<String, GrantState>>,
    next_token_id: AtomicU64,
}

impl ConfirmationService {
    /// Create a new empty confirmation service.
    pub fn new() -> Self {
        Self {
            grants: Mutex::new(HashMap::new()),
            next_token_id: AtomicU64::new(1),
        }
    }

    /// Create a confirmation grant bound to the given principal, action, channel,
    /// instance, and optional params. Returns the grant token.
    pub fn create_grant(
        &self,
        principal: AssertionPrincipal,
        action: impl Into<String>,
        channel: impl Into<String>,
        instance: impl Into<String>,
        params: HashMap<String, String>,
        ttl_secs: u64,
    ) -> String {
        let action = action.into();
        let channel = channel.into();
        let instance = instance.into();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let expires_at = now + ttl_secs;

        let id = self.next_token_id.fetch_add(1, Ordering::Relaxed);
        let token = format!("cg-{id}-{now:x}");

        let grant = ConfirmationGrant {
            token: token.clone(),
            principal,
            action,
            channel,
            instance,
            params,
            expires_at,
        };

        let mut grants = self.grants.lock().unwrap();
        grants.insert(
            token.clone(),
            GrantState {
                grant,
                redeemed: false,
            },
        );

        token
    }

    /// Redeem a confirmation grant.
    ///
    /// Validates:
    /// - Token exists and is not already redeemed.
    /// - Principal matches the grant's principal.
    /// - Channel matches the grant's channel.
    /// - Grant has not expired.
    /// - All supplied params match the grant's params.
    ///
    /// Returns the redeemed grant on success, or a `ConfirmationError` on
    /// failure.
    ///
    /// This method is atomic: concurrent redeems for the same token will
    /// result in exactly one success.
    pub fn redeem(
        &self,
        token: &str,
        principal: &AssertionPrincipal,
        channel: &str,
        params: &HashMap<String, String>,
    ) -> Result<ConfirmationGrant, ConfirmationError> {
        let mut grants = self.grants.lock().unwrap();

        let state = grants.get_mut(token).ok_or(ConfirmationError::NotFound)?;

        if state.redeemed {
            return Err(ConfirmationError::AlreadyRedeemed);
        }

        // Check expiry.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now > state.grant.expires_at {
            return Err(ConfirmationError::Expired);
        }

        // Check principal.
        if *principal != state.grant.principal {
            return Err(ConfirmationError::PrincipalMismatch);
        }

        // Check channel.
        if channel != state.grant.channel {
            return Err(ConfirmationError::ChannelMismatch);
        }

        // Check params — every param in the grant must match.
        for (key, expected) in &state.grant.params {
            match params.get(key) {
                Some(got) if got == expected => {}
                Some(got) => {
                    return Err(ConfirmationError::ParamsMismatch {
                        field: key.clone(),
                        expected: expected.clone(),
                        got: got.clone(),
                    });
                }
                None => {
                    return Err(ConfirmationError::ParamsMismatch {
                        field: key.clone(),
                        expected: expected.clone(),
                        got: String::new(),
                    });
                }
            }
        }

        // Mark as redeemed — single-use.
        state.redeemed = true;

        Ok(state.grant.clone())
    }
}

impl Default for ConfirmationService {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watchdog::auth::{AssertionPrincipal, FeishuPrincipal, FeishuVerificationMethod};
    use std::thread;

    // ─── Helpers ──────────────────────────────────────────────────────────

    fn test_principal(open_id: &str) -> AssertionPrincipal {
        AssertionPrincipal::Feishu(FeishuPrincipal {
            open_id: open_id.to_string(),
            chat_id: Some("chat_abc".to_string()),
            verified_by: FeishuVerificationMethod::CardAction,
        })
    }

    fn test_service() -> ConfirmationService {
        ConfirmationService::new()
    }

    fn make_grant(
        service: &ConfirmationService,
        principal: &AssertionPrincipal,
        channel: &str,
        ttl_secs: u64,
    ) -> String {
        let mut params = HashMap::new();
        params.insert("dry_run".to_string(), "true".to_string());
        service.create_grant(
            principal.clone(),
            "restart",
            channel,
            "svc_webui",
            params,
            ttl_secs,
        )
    }

    fn default_params() -> HashMap<String, String> {
        let mut p = HashMap::new();
        p.insert("dry_run".to_string(), "true".to_string());
        p
    }

    // ─── replay_rejected ──────────────────────────────────────────────────

    #[test]
    fn replay_rejected() {
        let service = test_service();
        let principal = test_principal("user_a");
        let token = make_grant(&service, &principal, "chat_1", 3600);
        let params = default_params();

        // First redeem must succeed.
        let result = service.redeem(&token, &principal, "chat_1", &params);
        assert!(result.is_ok(), "first redeem must succeed");

        // Second redeem (replay) must be rejected.
        let result = service.redeem(&token, &principal, "chat_1", &params);
        assert_eq!(result, Err(ConfirmationError::AlreadyRedeemed));
    }

    // ─── cross_user_forwarding_rejected ───────────────────────────────────

    #[test]
    fn cross_user_forwarding_rejected() {
        let service = test_service();
        let principal_a = test_principal("user_a");
        let principal_b = test_principal("user_b");
        let token = make_grant(&service, &principal_a, "chat_1", 3600);
        let params = default_params();

        // User B tries to redeem user A's grant — must be rejected.
        let result = service.redeem(&token, &principal_b, "chat_1", &params);
        assert_eq!(result, Err(ConfirmationError::PrincipalMismatch));
    }

    // ─── expired_grant_rejected ───────────────────────────────────────────

    #[test]
    fn expired_grant_rejected() {
        let service = test_service();
        let principal = test_principal("user_a");
        // Grant with ttl_secs = 0 expires immediately (issued_at == expires_at).
        let token = make_grant(&service, &principal, "chat_1", 0);
        // Sleep to ensure the clock has advanced past expires_at (second
        // granularity).
        thread::sleep(std::time::Duration::from_secs(1));

        let params = default_params();
        let result = service.redeem(&token, &principal, "chat_1", &params);
        assert_eq!(result, Err(ConfirmationError::Expired));
    }

    // ─── changed_params_rejected ──────────────────────────────────────────

    #[test]
    fn changed_params_rejected() {
        let service = test_service();
        let principal = test_principal("user_a");

        // Create a grant with two params.
        let mut grant_params = HashMap::new();
        grant_params.insert("dry_run".to_string(), "true".to_string());
        grant_params.insert("target".to_string(), "webui".to_string());

        let token = service.create_grant(
            principal.clone(),
            "restart",
            "chat_1",
            "svc_webui",
            grant_params,
            3600,
        );

        // Try to redeem with a wrong dry_run value.
        let mut wrong_params = HashMap::new();
        wrong_params.insert("dry_run".to_string(), "false".to_string());
        wrong_params.insert("target".to_string(), "webui".to_string());

        let result = service.redeem(&token, &principal, "chat_1", &wrong_params);
        assert_eq!(
            result,
            Err(ConfirmationError::ParamsMismatch {
                field: "dry_run".to_string(),
                expected: "true".to_string(),
                got: "false".to_string(),
            })
        );
    }

    // ─── concurrent_confirm_only_one_execution ────────────────────────────

    #[test]
    fn concurrent_confirm_only_one_execution() {
        let service = test_service();
        let principal = test_principal("user_a");
        let token = make_grant(&service, &principal, "chat_1", 3600);
        let params = default_params();

        let mut success_count = 0;
        let mut error_count = 0;

        // Spawn two threads that race to redeem the same token.
        thread::scope(|s| {
            let r1 = s.spawn(|| service.redeem(&token, &principal, "chat_1", &params));
            let r2 = s.spawn(|| service.redeem(&token, &principal, "chat_1", &params));

            let outcomes = [r1.join().unwrap(), r2.join().unwrap()];
            for outcome in &outcomes {
                match outcome {
                    Ok(_) => success_count += 1,
                    Err(e) => {
                        error_count += 1;
                        // The failing thread must see AlreadyRedeemed (not
                        // Expired or NotFound).
                        assert_eq!(*e, ConfirmationError::AlreadyRedeemed);
                    }
                }
            }
        });

        assert_eq!(success_count, 1, "exactly one redeem must succeed");
        assert_eq!(error_count, 1, "exactly one redeem must fail");
    }
}
