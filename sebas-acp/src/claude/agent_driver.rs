//! Spawn-time translation from `ProviderResolution` into the env vars and
//! CLI args the Anthropic `claude` CLI subprocess expects.
//!
//! `sebas` (`/provider` rework epic, bead `sebas-63f.2`) used to bake
//! per-agent env var names and CLI flags into the spawn call site. That
//! spread knowledge of "claude uses ANTHROPIC_BASE_URL" (or whatever) into
//! routing/orchestration code, where it had no business living.
//!
//! This module narrows the seam: `sebas` decides what semantics it wants
//! (`Off` / `Direct` / `Router`) and hands that to a driver; the driver
//! owns the agent-specific translation. Spawning code (`manager.rs` /
//! `ConnectConfig` plumbing) merges the driver's output into the env / argv
//! passed to the subprocess.
//!
//! The driver is currently a plain struct + inherent impl:
//! YAGNI says don't abstract until a second agent actually exists.
//! When (if) Codex / Gemini CLI / etc. lands, promote to a trait at that
//! seam — by then the type family will be informed by two real call sites.
//!
//! **注意**：Router 模式 agent 看到的永远是 Anthropic 协议面 ——
//! router 自身支持双协议（见 `sebas_router::proto::OPENAI_PATHS`），但仅
//! 服务于「外部 OpenAI 客户端直连 router」场景；sebas 自身用 Router
//! 模式时不可路由到 OpenAI-only provider。见 openspec/specs/provider-management/spec.md。
//!
//! Note: this file lives alongside `driver.rs`, which is the SDK engine
//! adapter (`CcDriver`) — the two share a name root but address different
//! concerns. This module is spawn-time configuration; `driver.rs` is the
//! post-spawn protocol loop.

/// Which upstream API protocol a `Direct` provider speaks.
///
/// Renamed from `Protocol` to disambiguate from
/// `sebas_router::proto::WireProtocol` (which carries the same meaning but at
/// the router→upstream seam, not the agent→upstream seam).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentProtocol {
    /// Anthropic Messages API (`/v1/messages`, x-api-key auth).
    Anthropic,
    /// OpenAI Chat Completions API (`/v1/chat/completions`, Bearer auth).
    OpenAi,
}

/// What `sebas` has decided about how this session should reach an LLM.
///
/// The driver turns this into the concrete env vars / argv the agent
/// subprocess consumes.
#[derive(Debug, Clone)]
pub enum ProviderResolution {
    /// Don't override anything — let the agent use whatever it has
    /// configured by default (its own env, its own config files).
    Off,
    /// Talk to an upstream provider directly: the agent gets the
    /// provider's URL and the provider's auth token.
    Direct {
        proto: AgentProtocol,
        base_url: String,
        auth_token: String,
    },
    /// Talk through sebas's router: the agent gets the router URL and a
    /// router-minted token. The router is what reaches the upstream.
    Router { url: String, auth_token: String },
    /// Provider resolution failed: configuration
    /// error — missing URL, unset `api_key_env`, unknown named provider,
    /// router mode without `[router]` config, etc. The driver emits a
    /// single in-band signal env var `SEBAS_PROVIDER_ERROR=<reason>` so
    /// the spawn wrapper can short-circuit (print + `exit(1)`) instead of
    /// silently launching claude with whatever it finds in its own
    /// environment.
    Error { reason: String },
}

/// Driver for the Anthropic `claude` CLI (a.k.a. Claude Code).
///
/// Translation table:
/// - `Off` → no env, no args; the CLI uses whatever it found in the
///   parent shell / its own config.
/// - `Direct { Anthropic, .. }` → `ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN`.
/// - `Direct { OpenAi, .. }` → `OPENAI_BASE_URL` + `OPENAI_API_KEY`.
/// - `Router { .. }` → `ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN`
///   pointing at the router. The router exposes itself as an
///   Anthropic-protocol endpoint regardless of which upstream it proxies
///   (the agent never has to know).
///
/// `resolve_env` and `resolve_args` are pure: same input → same output,
/// no I/O, no clock.
pub struct ClaudeCodeDriver;

impl ClaudeCodeDriver {
    /// Env vars to set in the subprocess environment, in the form
    /// `(NAME, value)` pairs. Order is not load-bearing.
    pub fn resolve_env(&self, r: &ProviderResolution) -> Vec<(String, String)> {
        match r {
            ProviderResolution::Off => Vec::new(),
            ProviderResolution::Direct {
                proto: AgentProtocol::Anthropic,
                base_url,
                auth_token,
            } => vec![
                ("ANTHROPIC_BASE_URL".to_string(), base_url.clone()),
                ("ANTHROPIC_AUTH_TOKEN".to_string(), auth_token.clone()),
            ],
            ProviderResolution::Direct {
                proto: AgentProtocol::OpenAi,
                base_url,
                auth_token,
            } => vec![
                ("OPENAI_BASE_URL".to_string(), base_url.clone()),
                ("OPENAI_API_KEY".to_string(), auth_token.clone()),
            ],
            // Router always rides the Anthropic protocol here — the
            // router itself presents an Anthropic-shaped API surface to
            // the agent, regardless of what's behind the router.
            ProviderResolution::Router { url, auth_token } => vec![
                ("ANTHROPIC_BASE_URL".to_string(), url.clone()),
                ("ANTHROPIC_AUTH_TOKEN".to_string(), auth_token.clone()),
            ],
            // The in-band error signal (openspec/specs/provider-management/spec.md)
            // that the spawn
            // wrapper intercepts. We don't pass any provider-shaped env
            // vars alongside — the agent must not see partial state.
            ProviderResolution::Error { reason } => {
                vec![("SEBAS_PROVIDER_ERROR".to_string(), reason.clone())]
            }
        }
    }

    /// Extra CLI args to append to the agent's argv. Empty when the agent
    /// takes its provider config exclusively from env.
    pub fn resolve_args(&self, _r: &ProviderResolution) -> Vec<String> {
        // The `claude` CLI takes its provider config exclusively from env;
        // no `--base-url` / `--api-key` / model flags are injected here
        // yet. Future: surface `--model <id>` from provider config.
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver() -> ClaudeCodeDriver {
        ClaudeCodeDriver
    }

    #[test]
    fn off_emits_no_env_and_no_args() {
        let d = driver();
        assert!(d.resolve_env(&ProviderResolution::Off).is_empty());
        assert!(d.resolve_args(&ProviderResolution::Off).is_empty());
    }

    #[test]
    fn direct_anthropic_emits_anthropic_env() {
        let d = driver();
        let env = d.resolve_env(&ProviderResolution::Direct {
            proto: AgentProtocol::Anthropic,
            base_url: "https://api.anthropic.com".into(),
            auth_token: "sk-ant-test".into(),
        });
        assert!(env.contains(&(
            "ANTHROPIC_BASE_URL".to_string(),
            "https://api.anthropic.com".to_string()
        )));
        assert!(env.contains(&(
            "ANTHROPIC_AUTH_TOKEN".to_string(),
            "sk-ant-test".to_string()
        )));
        assert!(!env.iter().any(|(k, _)| k.starts_with("OPENAI")));
    }

    #[test]
    fn direct_openai_emits_openai_env() {
        let d = driver();
        let env = d.resolve_env(&ProviderResolution::Direct {
            proto: AgentProtocol::OpenAi,
            base_url: "https://api.openai.com/v1".into(),
            auth_token: "sk-openai-test".into(),
        });
        assert!(env.contains(&(
            "OPENAI_BASE_URL".to_string(),
            "https://api.openai.com/v1".to_string()
        )));
        assert!(env.contains(&("OPENAI_API_KEY".to_string(), "sk-openai-test".to_string())));
        assert!(!env.iter().any(|(k, _)| k.starts_with("ANTHROPIC")));
    }

    #[test]
    fn router_uses_anthropic_env_pointing_at_router() {
        let d = driver();
        let env = d.resolve_env(&ProviderResolution::Router {
            url: "https://router.example/v1".into(),
            auth_token: "gw-tok".into(),
        });
        assert!(env.contains(&(
            "ANTHROPIC_BASE_URL".to_string(),
            "https://router.example/v1".to_string()
        )));
        assert!(env.contains(&("ANTHROPIC_AUTH_TOKEN".to_string(), "gw-tok".to_string())));
        // Router never leaks the OpenAI vars — the agent only sees the
        // Anthropic-shaped surface the router presents.
        assert!(!env.iter().any(|(k, _)| k.starts_with("OPENAI")));
    }

    #[test]
    fn no_variant_emits_args_yet() {
        // Sanity: every variant currently maps to empty argv. If a future
        // change adds args (e.g. --model), this test fails first and
        // forces a deliberate decision per variant.
        let d = driver();
        let variants = vec![
            ProviderResolution::Off,
            ProviderResolution::Direct {
                proto: AgentProtocol::Anthropic,
                base_url: "u".into(),
                auth_token: "t".into(),
            },
            ProviderResolution::Direct {
                proto: AgentProtocol::OpenAi,
                base_url: "u".into(),
                auth_token: "t".into(),
            },
            ProviderResolution::Router {
                url: "u".into(),
                auth_token: "t".into(),
            },
            ProviderResolution::Error {
                reason: "anything".into(),
            },
        ];
        for v in variants {
            assert!(
                d.resolve_args(&v).is_empty(),
                "{v:?} unexpectedly emitted args"
            );
        }
    }

    /// The `Error` variant (openspec/specs/provider-management/spec.md) is
    /// the in-band signal
    /// that the spawn wrapper intercepts. The driver must emit ONLY
    /// `SEBAS_PROVIDER_ERROR=<reason>` — no `ANTHROPIC_*` / `OPENAI_*`
    /// partial state, no spurious extra vars. The reason is what the
    /// wrapper will print to stderr before exiting non-zero.
    #[test]
    fn error_variant_emits_sebas_provider_error_env_only() {
        let d = driver();
        let env = d.resolve_env(&ProviderResolution::Error {
            reason: "provider 'deepseek' not found in overlay or router config".into(),
        });
        assert_eq!(
            env,
            vec![(
                "SEBAS_PROVIDER_ERROR".to_string(),
                "provider 'deepseek' not found in overlay or router config".to_string()
            )]
        );
        // No partial-state pollution: no provider-shaped env must leak
        // alongside the error signal.
        assert!(!env.iter().any(|(k, _)| k.starts_with("ANTHROPIC_")));
        assert!(!env.iter().any(|(k, _)| k.starts_with("OPENAI_")));
        // And args are still empty (nothing else to send to the agent).
        assert!(
            d.resolve_args(&ProviderResolution::Error { reason: "x".into() })
                .is_empty()
        );
    }
}
