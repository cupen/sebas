//! Agent-kind reachability probing (design D6/agent-driver spec "Reachability").
//!
//! Shared by the webui's `GET /api/agent-kinds` endpoint and the
//! `sebas agent-kinds list` CLI: one place owns the honest "is this agent's
//! binary present, and can it report a version" probe, so the create-session
//! dropdown and the CLI report the same thing. Reachability is advisory —
//! a missing binary reports `reachable=false` + a cause, never an error.

use async_trait::async_trait;
use serde::Serialize;

/// One agent in the catalog (`GET /api/agents`, workbench-agent-wire-fix
/// 3.1/3.2) — the single availability source for the frontend. The shape is
/// driver-free by contract: the driver is a configuration-layer concept and
/// never appears on the wire.
#[derive(Debug, Clone, Serialize)]
pub struct AgentKindInfo {
    /// The agent id on the wire: the `[acp.agents.*]` config key, or the
    /// reserved `"native"` for the built-in kernel.
    pub id: String,
    /// Product display name (config `display` field, derived from the driver
    /// when absent). Presentation only — never a wire identifier.
    pub display: String,
    /// Whether the agent can serve new sessions right now (binary probe for
    /// ACP agents; credential check for the native kernel).
    pub reachable: bool,
    /// Failure cause when unreachable (e.g. `"command not found"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The first line of `<exe> --version`, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// A configured agent to probe: its id, full launch argv, driver tag
/// (configuration-layer only, used to derive the display fallback) and the
/// optional explicit display name.
#[derive(Debug, Clone)]
pub struct AgentKindSource {
    pub slug: String,
    pub command: Vec<String>,
    /// 静态 launch 策略标签（配置层；不上 wire）：`"claude"` → display 兜底
    /// "Claude Code"，其余 → 键名本身。
    pub driver: String,
    pub display: Option<String>,
}

impl AgentKindSource {
    /// Display 兜底推导（D3）：显式配置优先；`claude` 驱动 → "Claude Code"；
    /// 其余 → agent id 本身。
    fn fallback_display(&self) -> String {
        match self.driver.as_str() {
            "claude" => "Claude Code".to_string(),
            _ => self.slug.clone(),
        }
    }
}

/// Probe one agent kind: presence via PATH/executable-bit check (the same
/// semantics as `config.rs::check_binary_reachable` — `command` is a shell
/// builtin, not a standalone binary, so we scan PATH directly), version via
/// `<exe> --version`. Pure-ish (no config knowledge); the binary crate
/// supplies the argv from `cfg.acp.agents`.
pub async fn discover_agent(source: &AgentKindSource) -> AgentKindInfo {
    let slug = source.slug.as_str();
    let display = source
        .display
        .clone()
        .unwrap_or_else(|| source.fallback_display());
    let command = source.command.as_slice();
    let Some(exe) = command.first().filter(|e| !e.is_empty()) else {
        return AgentKindInfo {
            id: slug.to_string(),
            display,
            reachable: false,
            cause: Some("empty command".to_string()),
            version: None,
        };
    };

    if !binary_reachable(exe) {
        return AgentKindInfo {
            id: slug.to_string(),
            display,
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
        id: slug.to_string(),
        display,
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
        out.push(discover_agent(src).await);
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
    fn source(slug: &str, command: &[&str]) -> AgentKindSource {
        AgentKindSource {
            slug: slug.to_string(),
            command: command.iter().map(|c| c.to_string()).collect(),
            driver: "acp".to_string(),
            display: None,
        }
    }

    #[tokio::test]
    async fn missing_binary_reports_command_not_found() {
        let info = discover_agent(&source(
            "gemini",
            &["sebas-nonexistent-binary-xyz-12345"],
        ))
        .await;
        assert!(!info.reachable);
        assert_eq!(info.cause.as_deref(), Some("command not found"));
        assert!(info.version.is_none());
    }

    /// 空 command（缺 argv[0]）报告 `empty command`，同样是不可达而非 panic。
    #[tokio::test]
    async fn empty_command_reports_empty_cause() {
        let info = discover_agent(&source("broken", &[])).await;
        assert!(!info.reachable);
        assert_eq!(info.cause.as_deref(), Some("empty command"));
    }

    /// workbench-agent-wire-fix 3.1：display 兜底——显式配置优先；
    /// `claude` 驱动 → "Claude Code"；其余 → agent id 本身。
    #[tokio::test]
    async fn display_falls_back_by_driver() {
        let mut src = source("myclaude", &["definitely-not-on-path-xyz"]);
        src.driver = "claude".to_string();
        assert_eq!(discover_agent(&src).await.display, "Claude Code");

        let mut src = source("codex", &["definitely-not-on-path-xyz"]);
        src.display = Some("Codex CLI".to_string());
        assert_eq!(discover_agent(&src).await.display, "Codex CLI");

        assert_eq!(discover_agent(&source("codex", &[])).await.display, "codex");
    }

    /// 一个必然存在的二进制（`sh`）应报告 reachable。
    #[tokio::test]
    async fn present_binary_reports_reachable() {
        let info = discover_agent(&source("shell", &["sh"])).await;
        assert!(info.reachable, "sh should be on PATH: {info:?}");
        assert!(info.cause.is_none());
    }

    /// opencode (`opencode acp`) 走现有 `discover_agent` 探测应兼容：二进制在
    /// PATH 时报告 reachable + 裸版本号（add-opencode-acp 的接入契约）。
    /// 二进制缺失时该测试自动跳过（不入失败），CI 无 opencode 也绿。
    #[tokio::test]
    async fn opencode_acp_probe_is_compatible() {
        let info = discover_agent(&source("opencode", &["opencode", "acp"])).await;
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
