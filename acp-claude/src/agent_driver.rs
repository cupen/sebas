//! `AgentDriver` — translate a `ProviderResolution` into the env vars and
//! CLI args a specific agent subprocess expects.
//!
//! `sebas` (`/provider` rework epic, bead `sebas-63f.2`) used to bake
//! per-agent env var names and CLI flags into the spawn call site. That
//! spread knowledge of "claude uses ANTHROPIC_BASE_URL" (or whatever) into
//! routing/orchestration code, where it had no business living.
//!
//! This module narrows the seam: `sebas` decides what semantics it wants
//! (`Off` / `Direct` / `Gateway`) and hands that to a driver; the driver
//! owns the agent-specific translation. Spawning code (`manager.rs` /
//! `ConnectConfig` plumbing) merges the driver's output into the env / argv
//! passed to the subprocess.
//!
//! New agents (Codex, Gemini CLI, future) implement `AgentDriver` without
//! `sebas` learning any of their idioms.
//!
//! Note: this file lives alongside `driver.rs`, which is the SDK engine
//! adapter (`CcDriver`) — the two share a name root but address different
//! concerns. This module is spawn-time configuration; `driver.rs` is the
//! post-spawn protocol loop.

/// Which upstream API protocol a `Direct` provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
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
        proto: Protocol,
        base_url: String,
        auth_token: String,
    },
    /// Talk through sebas's gateway: the agent gets the gateway URL and a
    /// gateway-minted token. The gateway is what reaches the upstream.
    Gateway {
        url: String,
        auth_token: String,
    },
}

/// Per-agent spawn-config adapter.
///
/// `id` is a stable slug the orchestrator can log / match on.
/// `resolve_env` and `resolve_args` are pure: same input → same output,
/// no I/O, no clock.
pub trait AgentDriver: Send + Sync {
    /// Stable identifier for this agent ("claude-code", "codex", ...).
    fn id(&self) -> &'static str;

    /// Env vars to set in the subprocess environment, in the form
    /// `(NAME, value)` pairs. Order is not load-bearing.
    fn resolve_env(&self, r: &ProviderResolution) -> Vec<(String, String)>;

    /// Extra CLI args to append to the agent's argv. Empty when the agent
    /// takes its provider config exclusively from env.
    fn resolve_args(&self, r: &ProviderResolution) -> Vec<String>;
}

/// Driver for the Anthropic `claude` CLI (a.k.a. Claude Code).
///
/// Translation table:
/// - `Off` → no env, no args; the CLI uses whatever it found in the
///   parent shell / its own config.
/// - `Direct { Anthropic, .. }` → `ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN`.
/// - `Direct { OpenAi, .. }` → `OPENAI_BASE_URL` + `OPENAI_API_KEY`.
/// - `Gateway { .. }` → `ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN`
///   pointing at the gateway. The gateway exposes itself as an
///   Anthropic-protocol endpoint regardless of which upstream it proxies
///   (the agent never has to know).
pub struct ClaudeCodeDriver;

impl AgentDriver for ClaudeCodeDriver {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn resolve_env(&self, r: &ProviderResolution) -> Vec<(String, String)> {
        match r {
            ProviderResolution::Off => Vec::new(),
            ProviderResolution::Direct {
                proto: Protocol::Anthropic,
                base_url,
                auth_token,
            } => vec![
                ("ANTHROPIC_BASE_URL".to_string(), base_url.clone()),
                ("ANTHROPIC_AUTH_TOKEN".to_string(), auth_token.clone()),
            ],
            ProviderResolution::Direct {
                proto: Protocol::OpenAi,
                base_url,
                auth_token,
            } => vec![
                ("OPENAI_BASE_URL".to_string(), base_url.clone()),
                ("OPENAI_API_KEY".to_string(), auth_token.clone()),
            ],
            // Gateway always rides the Anthropic protocol here — the
            // gateway itself presents an Anthropic-shaped API surface to
            // the agent, regardless of what's behind the gateway.
            ProviderResolution::Gateway { url, auth_token } => vec![
                ("ANTHROPIC_BASE_URL".to_string(), url.clone()),
                ("ANTHROPIC_AUTH_TOKEN".to_string(), auth_token.clone()),
            ],
        }
    }

    fn resolve_args(&self, _r: &ProviderResolution) -> Vec<String> {
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
    fn id_is_stable_slug() {
        assert_eq!(driver().id(), "claude-code");
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
            proto: Protocol::Anthropic,
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
            proto: Protocol::OpenAi,
            base_url: "https://api.openai.com/v1".into(),
            auth_token: "sk-openai-test".into(),
        });
        assert!(env.contains(&(
            "OPENAI_BASE_URL".to_string(),
            "https://api.openai.com/v1".to_string()
        )));
        assert!(env.contains(&(
            "OPENAI_API_KEY".to_string(),
            "sk-openai-test".to_string()
        )));
        assert!(!env.iter().any(|(k, _)| k.starts_with("ANTHROPIC")));
    }

    #[test]
    fn gateway_uses_anthropic_env_pointing_at_gateway() {
        let d = driver();
        let env = d.resolve_env(&ProviderResolution::Gateway {
            url: "https://gateway.example/v1".into(),
            auth_token: "gw-tok".into(),
        });
        assert!(env.contains(&(
            "ANTHROPIC_BASE_URL".to_string(),
            "https://gateway.example/v1".to_string()
        )));
        assert!(env.contains(&(
            "ANTHROPIC_AUTH_TOKEN".to_string(),
            "gw-tok".to_string()
        )));
        // Gateway never leaks the OpenAI vars — the agent only sees the
        // Anthropic-shaped surface the gateway presents.
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
                proto: Protocol::Anthropic,
                base_url: "u".into(),
                auth_token: "t".into(),
            },
            ProviderResolution::Direct {
                proto: Protocol::OpenAi,
                base_url: "u".into(),
                auth_token: "t".into(),
            },
            ProviderResolution::Gateway {
                url: "u".into(),
                auth_token: "t".into(),
            },
        ];
        for v in variants {
            assert!(d.resolve_args(&v).is_empty(), "{v:?} unexpectedly emitted args");
        }
    }
}