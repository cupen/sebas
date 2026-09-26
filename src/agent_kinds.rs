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

    // add-agent-settings-and-session-titles 3.2：目录 = **union**（与
    // webui `GET /api/agents` 同源）——store 行优先（store 是唯一权威，
    // 同 id 的 config 条目被取代），config-only 条目随后。
    let store_rows = crate::agent_store::load_store_rows_readonly();
    let mut sources: Vec<sebas_webui::agent_kinds::AgentKindSource> = Vec::new();
    // 墓碑行不进目录，但同 id 的 config 条目也一并排除（与 webui catalog
    // 同语义：删除对 config 种子 agent 同样生效）。
    let tombstoned: std::collections::BTreeSet<String> = store_rows
        .iter()
        .filter(|r| r.is_deleted())
        .map(|r| r.id.clone())
        .collect();
    for row in store_rows.iter().filter(|r| !r.is_deleted()) {
        let def = row.to_definition();
        sources.push(sebas_webui::agent_kinds::AgentKindSource {
            slug: row.id.clone(),
            command: def.command(),
            driver: def.driver.clone(),
            display: def.display.clone(),
        });
    }
    let config_agent_map = crate::agent_store::config_agent_map(&cfg);
    for (slug, def) in &config_agent_map {
        if sources.iter().any(|s| &s.slug == slug) || tombstoned.contains(slug) {
            continue;
        }
        sources.push(sebas_webui::agent_kinds::AgentKindSource {
            slug: slug.clone(),
            command: def.command(),
            driver: def.driver.clone(),
            display: def.display.clone(),
        });
    }

    let mut kinds: Vec<AgentKindInfo> = Vec::new();
    for src in &sources {
        kinds.push(discover_agent(src).await);
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
/// 用 `-` 占位），便于测试与未来扩展（如对齐 `sebas router list` 列布局）。
pub fn format_table(kinds: &[AgentKindInfo]) -> Vec<String> {
    kinds
        .iter()
        .map(|k| {
            format!(
                "{} {} {} {}",
                k.id,
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
        let info = discover_agent(&sebas_webui::agent_kinds::AgentKindSource {
            slug: "gemini".into(),
            command: vec!["sebas-nonexistent-binary-xyz-12345".into()],
            driver: "acp".into(),
            display: None,
        })
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
                id: "claude".into(),
                display: "Claude Code".into(),
                reachable: true,
                cause: None,
                version: Some("claude v1.2.3".into()),
            },
            AgentKindInfo {
                id: "gemini".into(),
                display: "gemini".into(),
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
