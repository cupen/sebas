//! Agent-kind reachability probing (design D6/agent-driver spec "Reachability").
//!
//! Shared by the webui's `GET /api/agent-kinds` endpoint and the
//! `sebas agent-kinds list` CLI: one place owns the honest "is this agent's
//! binary present, and can it report a version" probe, so the create-session
//! dropdown and the CLI report the same thing. Reachability is advisory —
//! a missing binary reports `reachable=false` + a cause, never an error.

use async_trait::async_trait;
use serde::Serialize;

/// One configured agent kind as the outside world sees it.
#[derive(Debug, Clone, Serialize)]
pub struct AgentKindInfo {
    /// Display name (currently the kind slug — no separate display name is
    /// configured).
    pub name: String,
    /// The open kind slug; the webui builds the `acp:<slug>` backend hint from it.
    pub slug: String,
    /// Whether the agent's binary is present and can report a version.
    pub reachable: bool,
    /// Failure cause when unreachable (e.g. `"command not found"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The first line of `<exe> --version`, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// A configured agent kind to probe: its open slug and full launch argv.
#[derive(Debug, Clone)]
pub struct AgentKindSource {
    pub slug: String,
    pub command: Vec<String>,
}

/// Probe one agent kind: presence via PATH/executable-bit check (the same
/// semantics as `config.rs::check_binary_reachable` — `command` is a shell
/// builtin, not a standalone binary, so we scan PATH directly), version via
/// `<exe> --version`. Pure-ish (no config knowledge); the binary crate
/// supplies the argv from `cfg.acp.agents`.
pub async fn discover_agent(slug: &str, command: &[String]) -> AgentKindInfo {
    let Some(exe) = command.first().filter(|e| !e.is_empty()) else {
        return AgentKindInfo {
            name: slug.to_string(),
            slug: slug.to_string(),
            reachable: false,
            cause: Some("empty command".to_string()),
            version: None,
        };
    };

    if !binary_reachable(exe) {
        return AgentKindInfo {
            name: slug.to_string(),
            slug: slug.to_string(),
            reachable: false,
            cause: Some("command not found".to_string()),
            version: None,
        };
    }

    // Version: `<exe> --version` (first non-empty line of stdout, falling back
    // to stderr). Failure to print a version is not fatal — presence already
    // proved reachability.
    let version = tokio::process::Command::new(exe)
        .arg("--version")
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if out.is_empty() {
                String::from_utf8_lossy(&o.stderr).trim().to_string()
            } else {
                out
            }
        })
        .filter(|s| !s.is_empty());

    AgentKindInfo {
        name: slug.to_string(),
        slug: slug.to_string(),
        reachable: true,
        cause: None,
        version,
    }
}

/// Whether `exe` resolves to an executable file: an absolute (or
/// slash-containing) path is checked directly; a bare name is resolved against
/// `$PATH`. Mirrors `config.rs::check_binary_reachable` semantics.
fn binary_reachable(exe: &str) -> bool {
    let is_executable = |p: &std::path::Path| -> bool {
        if !p.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(p)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            true
        }
    };

    if exe.contains('/') {
        is_executable(std::path::Path::new(exe))
    } else {
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(exe))))
            .unwrap_or(false)
    }
}

/// Discover every configured kind.
pub async fn discover_all(sources: &[AgentKindSource]) -> Vec<AgentKindInfo> {
    let mut out = Vec::with_capacity(sources.len());
    for src in sources {
        out.push(discover_agent(&src.slug, &src.command).await);
    }
    out
}

/// Supplies the agent-kind list to the webui server. The binary crate injects
/// the real provider (config-driven); tests inject a canned provider.
#[async_trait]
pub trait AgentKindProvider: Send + Sync {
    async fn agent_kinds(&self) -> Vec<AgentKindInfo>;
}

/// The production provider: probes each configured `AgentKindSource`.
pub struct ConfigAgentKindProvider {
    sources: Vec<AgentKindSource>,
}

impl ConfigAgentKindProvider {
    pub fn new(sources: Vec<AgentKindSource>) -> Self {
        Self { sources }
    }
}

#[async_trait]
impl AgentKindProvider for ConfigAgentKindProvider {
    async fn agent_kinds(&self) -> Vec<AgentKindInfo> {
        discover_all(&self.sources).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缺二进制时必须诚实报告 `reachable=false` + `cause="command not found"`
    /// （不 panic、不把错误当成功）。
    #[tokio::test]
    async fn missing_binary_reports_command_not_found() {
        let info = discover_agent(
            "gemini",
            &["sebas-nonexistent-binary-xyz-12345".to_string()],
        )
        .await;
        assert!(!info.reachable);
        assert_eq!(info.cause.as_deref(), Some("command not found"));
        assert!(info.version.is_none());
    }

    /// 空 command（缺 argv[0]）报告 `empty command`，同样是不可达而非 panic。
    #[tokio::test]
    async fn empty_command_reports_empty_cause() {
        let info = discover_agent("broken", &[]).await;
        assert!(!info.reachable);
        assert_eq!(info.cause.as_deref(), Some("empty command"));
    }

    /// 一个必然存在的二进制（`sh`）应报告 reachable。
    #[tokio::test]
    async fn present_binary_reports_reachable() {
        let info = discover_agent("shell", &["sh".to_string()]).await;
        assert!(info.reachable, "sh should be on PATH: {info:?}");
        assert!(info.cause.is_none());
    }

    /// opencode (`opencode acp`) 走现有 `discover_agent` 探测应兼容：二进制在
    /// PATH 时报告 reachable + 裸版本号（add-opencode-acp 的接入契约）。
    /// 二进制缺失时该测试自动跳过（不入失败），CI 无 opencode 也绿。
    #[tokio::test]
    async fn opencode_acp_probe_is_compatible() {
        let info = discover_agent("opencode", &["opencode".into(), "acp".into()]).await;
        if !info.reachable {
            eprintln!("opencode not on PATH; skipping opencode probe assertion");
            return;
        }
        assert!(info.cause.is_none(), "reachable opencode has no cause: {info:?}");
        let v = info.version.as_deref().unwrap_or_default();
        // opencode `--version` prints a bare semver like `1.18.25`.
        assert!(
            !v.is_empty() && v.chars().next().is_some_and(|c| c.is_ascii_digit()),
            "opencode version should be a bare version number, got {v:?}"
        );
    }
}
