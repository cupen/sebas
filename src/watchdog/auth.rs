//! Authorization primitives for the watchdog control plane.
//!
//! This module defines principals (who), signed assertions (a binding of
//! principal, action, and instance with expiry and replay protection), and a
//! verifier that checks assertion validity.
//!
//! # Cryptography note
//!
//! The baseline MAC provider (`DefaultMacProvider`) uses a simple keyed SHA-256
//! (concatenating key and payload). This is **NOT** a proper HMAC construction
//! and is not cryptographically secure. It is suitable for integration testing
//! and development environments. Replace with HMAC-SHA256 in production
//! deployments that face network-visible control surfaces.

use crate::watchdog::control::Actor;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Mutex;

// ─── Principals ────────────────────────────────────────────────────────────────

/// Identity proven by a Feishu (Lark) user through some verification channel.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeishuPrincipal {
    pub open_id: String,
    pub chat_id: Option<String>,
    /// Which channel performed the verification.
    pub verified_by: FeishuVerificationMethod,
}

/// How a Feishu principal was verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeishuVerificationMethod {
    /// Verified through a card action callback (signed by Feishu server).
    CardAction,
    /// Verified through a card query callback.
    CardQuery,
    /// Verified through an API token (tenant access token).
    ApiToken,
    /// Verified through an incoming webhook.
    Webhook,
}

/// Local Web UI principal — minimal identity for localhost-only sessions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WebUiPrincipal {
    pub session_id: String,
    /// Whether the session is bound to localhost (no remote network access).
    pub local: bool,
}

/// The actor identity carried inside an assertion.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AssertionPrincipal {
    Feishu(FeishuPrincipal),
    WebUi(WebUiPrincipal),
}

// ─── Signed Assertion ─────────────────────────────────────────────────────────

/// A signed assertion binding a principal, action, and instance identity.
///
/// The `mac_tag` authenticates all other fields plus a shared secret, providing
/// integrity and origin authentication. Each assertion carries a unique `nonce`
/// to prevent replay attacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedAssertion {
    pub principal: AssertionPrincipal,
    /// The action being authorized (e.g. "update", "rollback", "restart").
    pub action: String,
    /// The specific instance the action applies to (e.g. operation_id, service name).
    pub instance: String,
    /// Unix timestamp (seconds) when the assertion was issued.
    pub issued_at: u64,
    /// Unix timestamp (seconds) after which the assertion is invalid.
    pub expires_at: u64,
    /// Unique nonce to prevent replay.
    pub nonce: String,
    /// MAC tag authenticating the entire assertion.
    pub mac_tag: String,
}

// ─── MAC Provider ─────────────────────────────────────────────────────────────

/// Computes MAC tags for signed assertions.
pub trait MacProvider: Send + Sync {
    /// Compute a MAC tag for `payload` using `key`.
    fn compute(&self, payload: &str, key: &[u8]) -> String;
}

/// Baseline MAC provider using a simple keyed SHA-256.
///
/// This concatenates key and payload then SHA-256 hashes the result.
/// **This is NOT a proper HMAC.** Suitable for integration testing and
/// development environments. Replace with an HMAC-based provider in production.
#[derive(Debug, Default)]
pub struct DefaultMacProvider;

impl MacProvider for DefaultMacProvider {
    fn compute(&self, payload: &str, key: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(key);
        hasher.update(payload);
        hex::encode(hasher.finalize())
    }
}

// ─── Verification types ───────────────────────────────────────────────────────

/// Outcome of a successful verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedActor {
    pub actor: Actor,
    pub nonce: String,
}

/// Why a signed assertion was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectionReason {
    Expired,
    Replay(String),
    BadMac,
    ParameterMismatch {
        field: String,
        expected: String,
        got: String,
    },
}

impl std::fmt::Display for RejectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RejectionReason::Expired => write!(f, "assertion has expired"),
            RejectionReason::Replay(nonce) => write!(f, "replay detected (nonce: {nonce})"),
            RejectionReason::BadMac => write!(f, "MAC tag verification failed"),
            RejectionReason::ParameterMismatch {
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

// ─── Verifier trait ───────────────────────────────────────────────────────────

/// Verifies signed assertions.
pub trait ActorVerifier: Send + Sync {
    /// Verify a signed assertion and produce a verified actor.
    ///
    /// Checks performed:
    /// - MAC tag authenticity
    /// - Expiry (issued_at, expires_at, clock skew)
    /// - Replay protection (nonce uniqueness)
    fn verify(&self, assertion: &SignedAssertion) -> Result<VerifiedActor, RejectionReason>;
}

// ─── Default verifier ─────────────────────────────────────────────────────────

/// Configuration for the default verifier.
#[derive(Debug, Clone)]
pub struct VerifierConfig {
    pub mac_key: Vec<u8>,
    pub max_clock_skew_secs: u64,
}

impl Default for VerifierConfig {
    fn default() -> Self {
        Self {
            mac_key: Vec::new(),
            max_clock_skew_secs: 30,
        }
    }
}

/// Default verifier implementation.
///
/// Tracks used nonces in memory to prevent replay. Nonces are never evicted;
/// for long-lived processes, consider a bounded cache or periodic cleanup.
pub struct Verifier {
    config: VerifierConfig,
    mac_provider: Box<dyn MacProvider>,
    used_nonces: Mutex<HashSet<String>>,
}

impl Verifier {
    pub fn new(config: VerifierConfig, mac_provider: Box<dyn MacProvider>) -> Self {
        Self {
            config,
            mac_provider,
            used_nonces: Mutex::new(HashSet::new()),
        }
    }

    /// Convenience constructor using `DefaultMacProvider`.
    pub fn new_default(config: VerifierConfig) -> Self {
        Self::new(config, Box::new(DefaultMacProvider))
    }
}

impl ActorVerifier for Verifier {
    fn verify(&self, assertion: &SignedAssertion) -> Result<VerifiedActor, RejectionReason> {
        // 1. Check MAC tag integrity.
        let canonical = canonical_assertion_string(assertion);
        let expected = self.mac_provider.compute(&canonical, &self.config.mac_key);
        if assertion.mac_tag != expected {
            return Err(RejectionReason::BadMac);
        }

        // 2. Check expiry with clock skew allowance.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if now < assertion.issued_at.saturating_sub(self.config.max_clock_skew_secs) {
            return Err(RejectionReason::Expired);
        }
        if now > assertion.expires_at {
            return Err(RejectionReason::Expired);
        }

        // 3. Check replay nonce.
        {
            let mut nonces = self.used_nonces.lock().unwrap();
            if !nonces.insert(assertion.nonce.clone()) {
                return Err(RejectionReason::Replay(assertion.nonce.clone()));
            }
        }

        // 4. Convert to the control-layer Actor.
        let actor = principal_to_actor(&assertion.principal);

        Ok(VerifiedActor {
            actor,
            nonce: assertion.nonce.clone(),
        })
    }
}

// ─── Canonical representation ─────────────────────────────────────────────────

/// Deterministic string representation of a principal for MAC computation.
fn principal_mac_string(principal: &AssertionPrincipal) -> String {
    match principal {
        AssertionPrincipal::Feishu(p) => {
            format!(
                "feishu:{}:{}:{:?}",
                p.open_id,
                p.chat_id.as_deref().unwrap_or(""),
                p.verified_by
            )
        }
        AssertionPrincipal::WebUi(p) => format!("webui:{}:{}", p.session_id, p.local),
    }
}

/// Deterministic canonical string of all assertion fields for MAC computation.
fn canonical_assertion_string(assertion: &SignedAssertion) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        principal_mac_string(&assertion.principal),
        assertion.action,
        assertion.instance,
        assertion.issued_at,
        assertion.expires_at,
        assertion.nonce,
    )
}

// ─── Conversion helpers ───────────────────────────────────────────────────────

/// Convert an `AssertionPrincipal` to a `control::Actor`.
pub fn principal_to_actor(principal: &AssertionPrincipal) -> Actor {
    match principal {
        AssertionPrincipal::Feishu(p) => Actor::Feishu {
            open_id: p.open_id.clone(),
            chat_id: p.chat_id.clone(),
        },
        AssertionPrincipal::WebUi(p) => Actor::WebUi {
            user: Some(p.session_id.clone()),
            local: p.local,
        },
    }
}

/// Convert a `control::Actor` to an `AssertionPrincipal` if possible.
///
/// Returns `None` for `Actor::System` and `Actor::Cli` — these are not
/// assertion-based principals.
pub fn actor_to_principal(actor: &Actor) -> Option<AssertionPrincipal> {
    match actor {
        Actor::Feishu { open_id, chat_id } => Some(AssertionPrincipal::Feishu(FeishuPrincipal {
            open_id: open_id.clone(),
            chat_id: chat_id.clone(),
            verified_by: FeishuVerificationMethod::ApiToken,
        })),
        Actor::WebUi { user, local } => Some(AssertionPrincipal::WebUi(WebUiPrincipal {
            session_id: user.clone().unwrap_or_default(),
            local: *local,
        })),
        Actor::Cli { .. } | Actor::System => None,
    }
}

// ─── Assertion Builder ────────────────────────────────────────────────────────

/// Builder for creating signed assertions.
pub struct AssertionBuilder<'a> {
    principal: AssertionPrincipal,
    action: String,
    instance: String,
    mac_provider: &'a dyn MacProvider,
    mac_key: &'a [u8],
    ttl_secs: u64,
    nonce: Option<String>,
}

impl<'a> AssertionBuilder<'a> {
    pub fn new(
        principal: AssertionPrincipal,
        action: impl Into<String>,
        instance: impl Into<String>,
        mac_provider: &'a dyn MacProvider,
        mac_key: &'a [u8],
    ) -> Self {
        Self {
            principal,
            action: action.into(),
            instance: instance.into(),
            mac_provider,
            mac_key,
            ttl_secs: 300,
            nonce: None,
        }
    }

    pub fn ttl_secs(mut self, secs: u64) -> Self {
        self.ttl_secs = secs;
        self
    }

    pub fn nonce(mut self, nonce: impl Into<String>) -> Self {
        self.nonce = Some(nonce.into());
        self
    }

    pub fn build(self) -> SignedAssertion {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let nonce = self.nonce.unwrap_or_else(|| {
            // Deterministic nonce for baseline: process-id XOR timestamp.
            // Production should use a cryptographically secure random source.
            let mixed = now ^ (std::process::id() as u64);
            format!("n-{mixed:x}")
        });
        let issued_at = now;
        let expires_at = now + self.ttl_secs;

        let principal_repr = principal_mac_string(&self.principal);
        let canonical = format!(
            "{}|{}|{}|{}|{}|{}",
            principal_repr, self.action, self.instance, issued_at, expires_at, nonce,
        );
        let mac_tag = self.mac_provider.compute(&canonical, self.mac_key);

        SignedAssertion {
            principal: self.principal,
            action: self.action,
            instance: self.instance,
            issued_at,
            expires_at,
            nonce,
            mac_tag,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a verifier with a known key.
    fn test_verifier() -> Verifier {
        let config = VerifierConfig {
            mac_key: b"test-key".to_vec(),
            max_clock_skew_secs: 60,
        };
        Verifier::new_default(config)
    }

    /// Helper: create a feishu principal for testing.
    fn feishu_principal(open_id: &str) -> AssertionPrincipal {
        AssertionPrincipal::Feishu(FeishuPrincipal {
            open_id: open_id.to_string(),
            chat_id: Some("chat_abc".to_string()),
            verified_by: FeishuVerificationMethod::CardAction,
        })
    }

    /// Helper: build a valid assertion with a given nonce and TTL.
    fn make_valid_assertion(
        principal: AssertionPrincipal,
        action: &str,
        instance: &str,
        nonce: &str,
    ) -> SignedAssertion {
        let provider = DefaultMacProvider;
        let key = b"test-key";
        AssertionBuilder::new(principal, action, instance, &provider, key)
            .nonce(nonce)
            .ttl_secs(3600) // 1 hour, so it won't expire during the test
            .build()
    }

    // ─── forged_owner_id_rejected ─────────────────────────────────────────

    #[test]
    fn forged_owner_id_rejected() {
        let verifier = test_verifier();

        // Create a valid assertion for user_a.
        let mut assertion =
            make_valid_assertion(feishu_principal("user_a"), "update", "op_1", "nonce_1");

        // Tamper with the open_id — forge it to user_b without updating the MAC.
        assertion.principal = feishu_principal("user_b");

        // The forged assertion must be rejected.
        let result = verifier.verify(&assertion);
        assert_eq!(result, Err(RejectionReason::BadMac));
    }

    // ─── replayed_assertion_rejected ──────────────────────────────────────

    #[test]
    fn replayed_assertion_rejected() {
        let verifier = test_verifier();
        let assertion =
            make_valid_assertion(feishu_principal("user_a"), "update", "op_1", "unique_nonce_42");

        // First use should succeed.
        let first = verifier.verify(&assertion);
        assert!(first.is_ok(), "first verification must succeed");

        // Replay of the same assertion must be rejected.
        let second = verifier.verify(&assertion);
        assert_eq!(
            second,
            Err(RejectionReason::Replay("unique_nonce_42".to_string()))
        );
    }

    // ─── expired_assertion_rejected ───────────────────────────────────────

    #[test]
    fn expired_assertion_rejected() {
        let verifier = test_verifier();
        let provider = DefaultMacProvider;
        let key = b"test-key";

        // Create an assertion that expired 100 seconds ago.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let issued_at = now.saturating_sub(200);
        let expires_at = now.saturating_sub(100); // Already expired.

        let principal = feishu_principal("user_a");
        let principal_repr = principal_mac_string(&principal);
        let canonical = format!(
            "{}|{}|{}|{}|{}|{}",
            principal_repr, "update", "op_1", issued_at, expires_at, "nonce_exp",
        );
        let mac_tag = provider.compute(&canonical, key);

        let assertion = SignedAssertion {
            principal,
            action: "update".into(),
            instance: "op_1".into(),
            issued_at,
            expires_at,
            nonce: "nonce_exp".into(),
            mac_tag,
        };

        let result = verifier.verify(&assertion);
        assert_eq!(result, Err(RejectionReason::Expired));
    }

    // ─── parameter_substitution_rejected ──────────────────────────────────

    #[test]
    fn parameter_substitution_rejected() {
        let verifier = test_verifier();

        // Create a valid assertion for "update" action.
        let mut assertion =
            make_valid_assertion(feishu_principal("user_a"), "update", "op_1", "nonce_params");

        // Tamper with the action — substitute "rollback" for "update" without
        // updating the MAC.
        assertion.action = "rollback".to_string();

        let result = verifier.verify(&assertion);
        assert_eq!(result, Err(RejectionReason::BadMac));
    }

    // ─── RPC wire actor note ──────────────────────────────────────────────

    /// The RPC wire format (`RpcActor` in `control_rpc`) has no `System` variant.
    /// `Cli { uid }` and `Feishu { .. }` exist on the wire, but a remote caller
    /// cannot forge a `System`-level actor. See
    /// `control_rpc::tests::forged_system_actor_rejected`.
    ///
    /// This test verifies that the auth layer also cannot produce `System` or
    /// `Cli` actors from assertion principals, keeping the same invariant.
    #[test]
    fn rpc_wire_actor_has_no_system_variant() {
        // Feishu and WebUi actors can round-trip through the assertion layer.
        let feishu = Actor::Feishu {
            open_id: "u".into(),
            chat_id: None,
        };
        let webui = Actor::WebUi {
            user: None,
            local: true,
        };

        assert!(actor_to_principal(&feishu).is_some());
        assert!(actor_to_principal(&webui).is_some());

        // System and Cli have no assertion-based representation.
        assert!(actor_to_principal(&Actor::System).is_none());
        assert!(actor_to_principal(&Actor::Cli { uid: 0 }).is_none());
    }

    // ─── Conversion round-trip ────────────────────────────────────────────

    #[test]
    fn feishu_principal_round_trips_through_actor() {
        let principal = AssertionPrincipal::Feishu(FeishuPrincipal {
            open_id: "ou_abc123".into(),
            chat_id: Some("oc_def456".into()),
            verified_by: FeishuVerificationMethod::ApiToken,
        });

        let actor = principal_to_actor(&principal);
        let back = actor_to_principal(&actor).expect("must convert back");

        assert_eq!(back, principal);
    }

    #[test]
    fn webui_principal_round_trips_through_actor() {
        let principal = AssertionPrincipal::WebUi(WebUiPrincipal {
            session_id: "sess_xyz".into(),
            local: true,
        });

        let actor = principal_to_actor(&principal);
        let back = actor_to_principal(&actor).expect("must convert back");
        assert_eq!(back, principal);
    }
}