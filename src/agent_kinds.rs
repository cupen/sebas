//! `sebas agent-kinds list` — 报告每个已配置三方 agent 的可达性与版本。
//!
//! 探测逻辑共享自 `sebas_webui::agent_kinds`（同一份「命令存在 + `--version`
//! 探测」），保证 CLI 与 WebUI 创建会话下拉报出的可达性一致。输出形如
//! `slug reachable version cause?`，支持 `--json` 直接落 `AgentKindInfo`。

use crate::config::Config;
use crate::error::{Result, SebasError};
use sebas_webui::agent_kinds::{AgentKindInfo, discover_agent};

/// Arguments for `sebas agent-kinds list`.
pub struct ListArgs {
    pub config: String,
    pub json: bool,
}

/// CLI entry: read + parse config, probe each configured agent, print the table.
pub async fn run(args: ListArgs) -> Result<()> {
    let raw = std::fs::read_to_string(&args.config)
        .map_err(|e| SebasError::Config(format!("read config {}: {e}", args.config)))?;
    let cfg = Config::parse(&raw)?;

    let mut kinds: Vec<AgentKindInfo> = Vec::new();
    for slug in cfg.acp.agents.keys() {
        let command = cfg.acp.command_for(slug).unwrap_or_default();
        kinds.push(discover_agent(slug, &command).await);
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&kinds).expect("agent kinds always serialize")
        );
    } else {
        for line in format_table(&kinds) {
            println!("{line}");
        }
    }
    Ok(())
}

/// 纯函数：把探测结果格式化为 `slug reachable version cause?` 行（缺省字段
/// 用 `-` 占位），便于测试与未来扩展（如对齐 `sebas gateway list` 列布局）。
pub fn format_table(kinds: &[AgentKindInfo]) -> Vec<String> {
    kinds
        .iter()
        .map(|k| {
            format!(
                "{} {} {} {}",
                k.slug,
                k.reachable,
                k.version.as_deref().unwrap_or("-"),
                k.cause.as_deref().unwrap_or("-"),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缺二进制时诚实报告 `reachable=false` + `cause="command not found"`。
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

    /// 列布局：`slug reachable version cause`，缺省字段用 `-`。
    #[test]
    fn format_table_renders_slug_reachability_version_cause() {
        let kinds = vec![
            AgentKindInfo {
                name: "claude".into(),
                slug: "claude".into(),
                reachable: true,
                cause: None,
                version: Some("claude v1.2.3".into()),
            },
            AgentKindInfo {
                name: "gemini".into(),
                slug: "gemini".into(),
                reachable: false,
                cause: Some("command not found".into()),
                version: None,
            },
        ];
        assert_eq!(
            format_table(&kinds),
            vec![
                "claude true claude v1.2.3 -".to_string(),
                "gemini false - command not found".to_string(),
            ]
        );
    }
}
