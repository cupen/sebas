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
/// (configuration-layer only) and the optional explicit display name.
#[derive(Debug, Clone)]
pub struct AgentKindSource {
    pub slug: String,
    pub command: Vec<String>,
    /// 静态 launch 策略标签（配置层；不上 wire，display 兜底不再读它）。
    pub driver: String,
    pub display: Option<String>,
}

impl AgentKindSource {
    /// Display 兜底推导（fix-webui-qa-defects 7.1）：显式配置优先；缺省一律
    /// 回退 agent id 本身。此前 claude 驱动兜底 "Claude Code"——多个 claude
    /// 驱动的 agent 在下拉里三项同名、无法分辨；id 是唯一稳定的区分词。
    fn fallback_display(&self) -> String {
        self.slug.clone()
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
    // proved reachability. Probe the RESOLVED path: on Windows a bare name may
    // only exist as a PATHEXT-suffixed file (`opencode.cmd`), which a raw
    // spawn of the bare name would miss.
    let version = tokio::process::Command::new(resolved_binary(exe).unwrap_or_else(|| exe.into()))
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
    resolved_binary(exe).is_some()
}

/// Resolve `exe` to a spawnable file path: a slash-containing path is checked
/// directly; a bare name is resolved against `$PATH`. Unix checks the
/// executable bit; Windows follows the CreateProcess naming — bare names
/// gain PATHEXT suffixes (`sh` → `sh.exe`, `opencode` → `opencode.cmd`) and
/// extensionless files (npm shims) are skipped, they cannot be spawned.
fn resolved_binary(exe: &str) -> Option<std::path::PathBuf> {
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

    if exe.contains('/') || exe.contains('\\') {
        let p = std::path::Path::new(exe);
        return is_executable(p).then(|| p.to_path_buf());
    }

    let dirs = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    #[cfg(unix)]
    return dirs.into_iter().map(|dir| dir.join(exe)).find(|p| is_executable(p));
    #[cfg(windows)]
    {
        let exts: Vec<String> = if std::path::Path::new(exe).extension().is_some() {
            vec![String::new()]
        } else {
            std::env::var_os("PATHEXT")
                .map(|v| {
                    v.to_string_lossy()
                        .split(';')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_ascii_lowercase())
                        .collect()
                })
                .unwrap_or_else(|| vec![".exe".to_string()])
        };
        for dir in dirs {
            for ext in &exts {
                let cand = dir.join(format!("{exe}{ext}"));
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
        None
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

/// agents 域快照条目（`AgentRow::to_item` 投影）→ 探测源（
/// add-agent-settings-and-session-titles 3.2）：claude → `[path]`（缺省
/// `"claude"`）；acp → `[path, args..]`。`driver` 是配置层标签，只进探测
/// 源、不上 catalog wire。
pub fn source_from_store_item(item: &serde_json::Value) -> Option<AgentKindSource> {
    let id = item.get("id")?.as_str()?.to_string();
    let driver = item
        .get("driver")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("acp")
        .to_string();
    let path = item
        .get("path")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if driver == "claude" {
                "claude".to_string()
            } else {
                String::new()
            }
        });
    let mut command = vec![path];
    if let Some(args) = item.get("args").and_then(serde_json::Value::as_array) {
        command.extend(
            args.iter()
                .filter_map(|a| a.as_str())
                .map(str::to_string),
        );
    }
    if command[0].is_empty() {
        command.remove(0);
    }
    Some(AgentKindSource {
        slug: id,
        command,
        driver,
        display: item
            .get("display")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    })
}

/// Supplies the agent-kind list to the webui server. The binary crate injects
/// the real provider (config-driven); tests inject a canned provider.
#[async_trait]
pub trait AgentKindProvider: Send + Sync {
    async fn agent_kinds(&self) -> Vec<AgentKindInfo>;
    /// The kind a new session gets when the operator does not request one:
    /// config `[acp] default` at the assembly point, with the same historical
    /// `"claude"` fallback `AcpConfig::default_kind` applies when unset.
    /// Surfaced read-only on `/api/about` (About INSTANCE segment,
    /// preselect-last-used-model 3.2) — never invented there.
    fn default_agent_kind(&self) -> String;
}

/// The production provider: probes each configured `AgentKindSource`.
pub struct ConfigAgentKindProvider {
    sources: Vec<AgentKindSource>,
    default_kind: String,
}

impl ConfigAgentKindProvider {
    /// Minimal assemblies without config context (tests, bare servers): the
    /// fallback mirrors `AcpConfig::default_kind`'s own unset default, so the
    /// reported value is the product's real semantic default, not a guess.
    pub fn new(sources: Vec<AgentKindSource>) -> Self {
        Self {
            sources,
            default_kind: "claude".to_string(),
        }
    }

    /// Config-driven production form（webui_cmd / run 装配点）：显式传入
    /// `cfg.acp.default_kind().to_string()`。
    pub fn with_default_kind(sources: Vec<AgentKindSource>, default_kind: String) -> Self {
        Self {
            sources,
            default_kind,
        }
    }
}

#[async_trait]
impl AgentKindProvider for ConfigAgentKindProvider {
    async fn agent_kinds(&self) -> Vec<AgentKindInfo> {
        discover_all(&self.sources).await
    }

    fn default_agent_kind(&self) -> String {
        self.default_kind.clone()
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
        let info = discover_agent(&source("gemini", &["sebas-nonexistent-binary-xyz-12345"])).await;
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

    /// fix-webui-qa-defects 7.1：display 兜底——显式配置优先；缺省一律
    /// 回退 agent id（不再按 driver 推导——多个 claude 驱动的 agent 曾
    /// 三项同名 "Claude Code"）。
    #[tokio::test]
    async fn display_falls_back_to_the_agent_id() {
        let mut src = source("myclaude", &["definitely-not-on-path-xyz"]);
        src.driver = "claude".to_string();
        assert_eq!(
            discover_agent(&src).await.display,
            "myclaude",
            "no display config must fall back to the agent id, not a driver label"
        );

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

    /// store 快照条目 → 探测源：claude 缺 path 回退内置 `claude`；acp 组装
    /// 完整 argv；空 argv 如实保留（探测报 empty command）。
    #[test]
    fn store_item_maps_to_probe_source() {
        let claude = serde_json::json!({"id": "seeded", "driver": "claude"});
        let src = source_from_store_item(&claude).unwrap();
        assert_eq!(src.command, vec!["claude".to_string()], "claude 缺 path 走内置");
        assert_eq!(src.driver, "claude");

        let acp = serde_json::json!({
            "id": "cursor",
            "driver": "acp",
            "path": "cursor-agent",
            "args": ["acp"],
            "display": "Cursor",
        });
        let src = source_from_store_item(&acp).unwrap();
        assert_eq!(src.command, vec!["cursor-agent".to_string(), "acp".to_string()]);
        assert_eq!(src.display.as_deref(), Some("Cursor"));

        let empty = serde_json::json!({"id": "broken", "driver": "acp"});
        let src = source_from_store_item(&empty).unwrap();
        assert!(src.command.is_empty(), "acp 无 argv[0] = 空 argv");
        assert!(source_from_store_item(&serde_json::json!({"driver": "acp"})).is_none(),
            "缺 id 的条目不成源");
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
        assert!(
            info.cause.is_none(),
            "reachable opencode has no cause: {info:?}"
        );
        let v = info.version.as_deref().unwrap_or_default();
        // opencode `--version` 曾输出裸 semver（`1.18.25`）；≥2.x 起改为
        // `opencode v2.0.8` 前缀形。探测契约（add-opencode-acp）只要求
        // reachable + 非空版本串——断言「含数字的非空版本」，两种形态都过。
        assert!(
            !v.is_empty() && v.chars().any(|c| c.is_ascii_digit()),
            "opencode version should be a non-empty version string, got {v:?}"
        );
    }
}
