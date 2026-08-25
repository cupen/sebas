//! Authorization primitives for the watchdog control plane.
//!
//! 鉴权模型本身在 control RPC 层：Unix socket 权限（0600）+ 每实例启动 secret +
//! envelope 内的 actor 字段（System 不可从线上伪造）。本模块只承载确认流所需的
//! principal 类型，以及 principal 与 `control::Actor` 的互转。

use crate::watchdog::control::Actor;

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

/// The actor identity bound into a dangerous-action confirmation grant.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AssertionPrincipal {
    Feishu(FeishuPrincipal),
    WebUi(WebUiPrincipal),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_principal_roundtrip_feishu() {
        let actor = Actor::Feishu {
            open_id: "ou_1".into(),
            chat_id: Some("oc_2".into()),
        };
        let principal = actor_to_principal(&actor).expect("feishu maps to principal");
        assert_eq!(principal_to_actor(&principal), actor);
    }

    #[test]
    fn actor_principal_roundtrip_webui() {
        let actor = Actor::WebUi {
            user: Some("session-1".into()),
            local: true,
        };
        let principal = actor_to_principal(&actor).expect("webui maps to principal");
        assert_eq!(principal_to_actor(&principal), actor);
    }

    #[test]
    fn cli_and_system_have_no_principal() {
        assert!(actor_to_principal(&Actor::Cli { uid: 1000 }).is_none());
        assert!(actor_to_principal(&Actor::System).is_none());
    }
}
