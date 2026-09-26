//! Agent 目录的 store 侧 glue（add-agent-settings-and-session-titles）。
//!
//! config.toml 的 `[acp.agents.*]` **BREAKING** 降级为种子源（design 决策
//! 2）：settings.db 的 `agents` 表是 agent 目录唯一运行时权威，本模块负责
//! 三件事——
//!
//! 1. **种子导入**（[`seed_agents_from_config`]）：core 启动时把 config 里
//!    db 缺失的 id 幂等建成行（`source=seed`）；同 id 已存在 → store 赢 +
//!    启动 notice 指向 Settings。放在状态库 init 之前（与 legacy defaults
//!    导入同一时机，经同一 writer 句柄串行提交）。独立 webui 进程不开库
//!    （决策 3「webui 不开库」），种子导入只发生在 core 进程。
//! 2. **spawn 动态解析**（[`resolve_agent_config`]）：config 注册表 miss →
//!    读 agents 表构建等价 `AgentConfig`，两处皆无 → `None`（调用方给
//!    typed unknown-agent 拒绝）。每次 spawn 直读，无缓存失效协议。
//! 3. **注册表登记**（[`ensure_registered`]）：store 行解析出的定义 upsert
//!    进 `SessionManager` 的 kind → driver 注册表（claude → 专属驱动 +
//!    模型别名表，acp → 通用 ACP 驱动），spawn 路径据此拿驱动实例。

use crate::config::{AgentConfig, AcpClaudeConfig, Config};
use sebas_acp::claude::manager::{AgentEntry, SessionManager};
use sebas_models::agent::AgentDefinition;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// `AgentConfig` → 等价 `AgentDefinition`（种子导入的载荷半边）。
pub fn definition_of(agent: &AgentConfig) -> AgentDefinition {
    match agent {
        AgentConfig::Claude(c) => AgentDefinition {
            driver: "claude".to_string(),
            path: Some(c.path.clone()),
            args: c.args.clone(),
            display: c.display.clone(),
            models: c.models.clone(),
            startup_timeout_secs: c.startup_timeout_secs,
            idle_kill_secs: c.idle_kill_secs,
            work_dir: c.work_dir.clone(),
        },
        AgentConfig::Acp {
            command,
            startup_timeout_secs,
            idle_kill_secs,
            display,
        } => {
            let (path, args) = match command.split_first() {
                Some((head, rest)) => (Some(head.clone()), rest.to_vec()),
                None => (None, Vec::new()),
            };
            AgentDefinition {
                driver: "acp".to_string(),
                path,
                args,
                display: display.clone(),
                models: None,
                startup_timeout_secs: *startup_timeout_secs,
                idle_kill_secs: *idle_kill_secs,
                work_dir: None,
            }
        }
    }
}

/// store 行 → 等价 `AgentConfig`（spawn 动态解析的载荷半边）。
pub fn config_of_definition(def: &AgentDefinition) -> AgentConfig {
    match def.driver.as_str() {
        "claude" => AgentConfig::Claude(AcpClaudeConfig {
            path: def.path.clone().unwrap_or_else(|| {
                sebas_models::agent::DEFAULT_CLAUDE_PATH.to_string()
            }),
            args: def.args.clone(),
            display: def.display.clone(),
            // sessions_dir 不入表（决策 1）：缺省走既有默认。
            sessions_dir: "~/.claude/sessions".to_string(),
            work_dir: def.work_dir.clone(),
            startup_timeout_secs: def.startup_timeout_secs,
            idle_kill_secs: def.idle_kill_secs,
            models: def.models.clone(),
        }),
        _ => AgentConfig::Acp {
            command: def.command(),
            startup_timeout_secs: def.startup_timeout_secs.max(1),
            idle_kill_secs: def.idle_kill_secs,
            display: def.display.clone(),
        },
    }
}

/// 种子导入（design 决策 2）：config `[acp.agents.*]` 中 db 缺失的 id 建
/// `AgentRow`（source=seed），同 id 已存在跳过 + notice。幂等——二次启动
/// 零变更。失败不阻断启动（种子只是目录的补全，warn 点名即可）。
pub async fn seed_agents_from_config(handle: &crate::sebas_state::writer::StateHandle, cfg: &Config) {
    let Ok(existing) = handle
        .exec(crate::sebas_state::repo::load_agents)
        .await
    else {
        tracing::warn!("agents 种子导入读取 agents 表失败（不阻断启动）");
        return;
    };
    for (slug, agent_cfg) in cfg.acp.agents.iter() {
        if slug == sebas_models::agent::RESERVED_NATIVE_ID {
            continue;
        }
        if existing.iter().any(|r| r.id == *slug && r.is_deleted()) {
            tracing::info!(
                agent = %slug,
                "config [acp.agents.{slug}] 对应 id 已被操作员删除（墓碑留存）：跳过回填"
            );
            continue;
        }
        if existing.iter().any(|r| r.id == *slug) {
            // store 赢：Settings 里管理过的同名条目优先，config 段被忽略。
            tracing::info!(
                agent = %slug,
                "config [acp.agents.{slug}] 与 agents 库现有条目同 id：以 Settings 管理的 store 行为准，config 条目忽略（可在 Settings 查看/编辑）"
            );
            continue;
        }
        let row = sebas_models::agent::AgentRow::from_definition(
            slug,
            &definition_of(agent_cfg),
            "seed",
        );
        if let Err(e) = handle
            .exec(move |conn| crate::sebas_state::repo::save_agent(conn, row))
            .await
        {
            tracing::warn!(agent = %slug, error = %e, "agents 种子导入写行失败（不阻断启动）");
        } else {
            tracing::info!(agent = %slug, "config agent 已种子导入 agents 库（source=seed）");
        }
    }
}

/// spawn 动态解析（决策 3）：config 注册表优先，miss 则读 agents 表构建
/// 等价 `AgentConfig`。两处皆无 → `None`（调用方给 typed unknown-agent
/// 拒绝）。每次 spawn 直读（本地 SQLite 单行查，微秒级）。墓碑守卫：id
/// 在 store 留有删除墓碑时 config 臂同样视为缺席——UI 删除的种子 agent
/// 不经 config 复活（spec「delete removes the agent for new sessions」）。
pub async fn resolve_agent_config(cfg: &Config, kind: &str) -> Option<AgentConfig> {
    if kind.is_empty() {
        let fallback = cfg.acp.default_kind().to_string();
        return Box::pin(resolve_agent_config(cfg, &fallback)).await;
    }
    let engine = sebas_dispatch::state_store::engine();
    let store_rows = match &engine {
        Some(engine) => engine.load_agents().await.ok(),
        None => None,
    };
    let tombstoned = |id: &str| {
        store_rows
            .as_ref()
            .is_some_and(|rows| rows.iter().any(|r| r.id == id && r.is_deleted()))
    };
    if let Some(configured) = cfg.acp.agents.get(kind) {
        if tombstoned(kind) {
            return None;
        }
        return Some(configured.clone());
    }
    let rows = store_rows?;
    rows.iter()
        .find(|r| r.id == kind && !r.is_deleted())
        .map(|r| config_of_definition(&r.to_definition()))
}

/// 把解析出的 agent 定义 upsert 进 `SessionManager` 的注册表（store-only
/// agent 的驱动实例在此就位；config agent 启动时已在注册表里，重登记是无
/// 害覆盖）。返回该 kind 的 startup timeout。
pub fn ensure_registered(mgr: &Arc<SessionManager>, kind: &str, agent: &AgentConfig) -> Duration {
    let driver: Arc<dyn sebas_acp::AgentDriver> = match agent {
        // 与 build_agent_registry 同构：claude 驱动携带各自的模型别名表。
        AgentConfig::Claude(c) => {
            Arc::new(sebas_acp::ClaudeDriver::with_models(c.resolved_models()))
        }
        AgentConfig::Acp { .. } => Arc::new(sebas_acp::AcpDriver),
    };
    let startup_timeout = agent.startup_timeout();
    mgr.upsert_agent(
        kind,
        AgentEntry {
            driver,
            startup_timeout,
        },
    );
    startup_timeout
}

/// config `[acp.agents.*]` 全量的 launch 定义视图（agent-kinds CLI 同源
/// union 的 store 侧补充用）。
pub fn config_agent_map(cfg: &Config) -> BTreeMap<String, AgentDefinition> {
    cfg.acp
        .agents
        .iter()
        .map(|(slug, a)| (slug.clone(), definition_of(a)))
        .collect()
}

/// 只读读出状态库 agents 表（`sebas agent-kinds list` 等 CLI 面的 union
/// 数据源；core 进程之外没有 state store 引擎，也不允许对权威库做任何写）。
/// 库打不开 / 表未建（旧状态目录）→ 空表（调用方如实退化为 config-only
/// 目录）。
pub fn load_store_rows_readonly() -> Vec<sebas_models::agent::AgentRow> {
    let path = sebas_domain::state_paths::Database::Settings.resolve();
    let Ok(mut conn) = sebas_db::conn::open_readonly(&path) else {
        return Vec::new();
    };
    crate::sebas_state::repo::load_agents(&mut conn).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_config(path: &str) -> AgentConfig {
        AgentConfig::Claude(AcpClaudeConfig {
            path: path.to_string(),
            ..AcpClaudeConfig::default()
        })
    }

    /// `AgentConfig` ↔ `AgentDefinition` 双向转换：claude 与 acp 两个变体
    /// 的 launch 定义逐字段保真。
    #[test]
    fn definition_conversion_preserves_launch_fields() {
        let claude = claude_config("/opt/claude");
        let def = definition_of(&claude);
        assert_eq!(def.driver, "claude");
        assert_eq!(def.path.as_deref(), Some("/opt/claude"));
        match config_of_definition(&def) {
            AgentConfig::Claude(c) => assert_eq!(c.path, "/opt/claude"),
            other => panic!("round trip must stay claude: {other:?}"),
        }

        let acp = AgentConfig::Acp {
            command: vec!["cursor-agent".into(), "acp".into()],
            startup_timeout_secs: 45,
            idle_kill_secs: 0,
            display: Some("Cursor".into()),
        };
        let def = definition_of(&acp);
        assert_eq!(def.driver, "acp");
        assert_eq!(def.path.as_deref(), Some("cursor-agent"));
        assert_eq!(def.args, vec!["acp".to_string()]);
        assert_eq!(def.display.as_deref(), Some("Cursor"));
        match config_of_definition(&def) {
            AgentConfig::Acp {
                command,
                startup_timeout_secs,
                idle_kill_secs,
                display,
            } => {
                assert_eq!(command, vec!["cursor-agent".to_string(), "acp".into()]);
                assert_eq!(startup_timeout_secs, 45);
                assert_eq!(idle_kill_secs, 0);
                assert_eq!(display.as_deref(), Some("Cursor"));
            }
            other => panic!("round trip must stay acp: {other:?}"),
        }
    }

    /// 种子导入三场景（spec「Config agents seed the store」）：fresh 目录
    /// 导入全量；二次启动幂等零变更；同 id 冲突 store 赢 + 打 notice。
    #[tokio::test]
    async fn seeding_is_idempotent_and_store_wins() {
        use crate::sebas_state::writer::StateWriter;
        let dir = tempfile::tempdir().unwrap();
        let writer = StateWriter::start_settings(dir.path().join("settings.db")).unwrap();
        let cfg: Config = toml::from_str(
            r#"
            [acp]
            default = "claude"

            [acp.agents.claude]
            driver = "claude"
            path = "claude"

            [acp.agents.cursor]
            driver = "acp"
            command = ["cursor-agent", "acp"]
            "#,
        )
        .unwrap();

        // 第一次：fresh 目录全量导入。
        seed_agents_from_config(writer.handle(), &cfg).await;
        let rows = writer
            .handle()
            .exec(crate::sebas_state::repo::load_agents)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "fresh 导入全量");
        assert!(rows.iter().all(|r| r.source == "seed"));

        // 操作员经 Settings 改了 claude 行（store 赢的前提）。
        let edited = {
            let mut r = rows.iter().find(|r| r.id == "claude").unwrap().clone();
            r.display = Some("Operator Edited".into());
            r
        };
        writer
            .handle()
            .exec(move |conn| crate::sebas_state::repo::save_agent(conn, edited))
            .await
            .unwrap();

        // 第二次启动：幂等——零新行、store 行不动。
        seed_agents_from_config(writer.handle(), &cfg).await;
        let rows = writer
            .handle()
            .exec(crate::sebas_state::repo::load_agents)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "幂等：不产生重复行");
        let claude = rows.iter().find(|r| r.id == "claude").unwrap();
        assert_eq!(
            claude.display.as_deref(),
            Some("Operator Edited"),
            "同 id 冲突 store 赢"
        );
    }

    /// spawn 动态解析：config 命中优先返回 config 形态；miss 读 store；
    /// 两处皆无返回 None（typed unknown-agent 拒绝的调用方前提）。
    #[tokio::test]
    async fn resolve_prefers_config_then_store() {
        let _engine_guard = sebas_dispatch::test_engine::install_fresh();
        let cfg: Config = toml::from_str(
            r#"
            [acp.agents.claude]
            driver = "claude"
            path = "claude"
            "#,
        )
        .unwrap();

        // config 命中。
        let hit = resolve_agent_config(&cfg, "claude").await.unwrap();
        assert!(matches!(hit, AgentConfig::Claude(_)));
        // store miss + config miss。
        assert!(resolve_agent_config(&cfg, "ghost").await.is_none());

        // store 行就位后 miss 变 hit（经全局引擎的 agents 域 mutation 写入）。
        let engine = sebas_dispatch::state_store::engine().unwrap();
        sebas_dispatch::state_store::agents_mutation(
            engine,
            &serde_json::json!({
                "op": "put",
                "id": "opencode",
                "agent": {"driver": "acp", "path": "opencode", "args": ["acp"]},
            }),
        )
        .await
        .unwrap();
        let store_hit = resolve_agent_config(&cfg, "opencode").await.unwrap();
        match store_hit {
            AgentConfig::Acp { command, .. } => {
                assert_eq!(command, vec!["opencode".to_string(), "acp".into()])
            }
            other => panic!("store row resolves to acp config: {other:?}"),
        }
    }

    /// `ensure_registered` 把 store-only kind 登记进 SessionManager：登记后
    /// `create_session` 不再报 unknown agent kind（驱动选择可用的前提）。
    #[tokio::test]
    async fn ensure_registered_makes_store_kind_known_to_the_manager() {
        let mgr = Arc::new(SessionManager::new("claude".into(), Default::default()));
        let agent = config_of_definition(&AgentDefinition {
            driver: "acp".into(),
            path: Some("opencode".into()),
            args: vec!["acp".into()],
            ..Default::default()
        });
        assert!(!mgr.has_agent("opencode"), "登记前 unknown");
        ensure_registered(&mgr, "opencode", &agent);
        assert!(mgr.has_agent("opencode"), "登记后 registry 命中");
    }

    /// 删除墓碑闭环（内存引擎侧）：config 种子 agent 被 UI 删除后——
    /// ① spawn 解析不再经 config 臂复活（None）；② 同 id 重新 put = 复活
    /// （解析恢复）。种子导入侧的墓碑跳过由
    /// `reseed_skips_tombstoned_rows` 单独钉。
    #[tokio::test]
    async fn deleted_config_agent_stays_deleted_until_recreated() {
        let _engine_guard = sebas_dispatch::test_engine::install_fresh();
        let engine = sebas_dispatch::state_store::engine().unwrap();
        let cfg: Config = toml::from_str(
            r#"
            [acp.agents.claude]
            driver = "claude"
            path = "claude"
            "#,
        )
        .unwrap();

        // 删除种子 agent（软删：行留墓碑）。
        sebas_dispatch::state_store::agents_mutation(
            engine,
            &serde_json::json!({
                "op": "put",
                "id": "claude",
                "agent": {"driver": "claude", "path": "claude"},
            }),
        )
        .await
        .unwrap();
        assert!(
            resolve_agent_config(&cfg, "claude").await.is_some(),
            "活跃 store 行 + config 命中 → 正常解析"
        );

        sebas_dispatch::state_store::agents_mutation(
            engine,
            &serde_json::json!({"op": "delete", "id": "claude"}),
        )
        .await
        .unwrap();
        assert!(
            resolve_agent_config(&cfg, "claude").await.is_none(),
            "墓碑 id 的 spawn 解析 = None（config 臂也被守卫拦下）"
        );

        // 同 id 重新创建（put）= 复活。
        sebas_dispatch::state_store::agents_mutation(
            engine,
            &serde_json::json!({
                "op": "put",
                "id": "claude",
                "agent": {"driver": "claude", "path": "claude"},
            }),
        )
        .await
        .unwrap();
        assert!(
            resolve_agent_config(&cfg, "claude").await.is_some(),
            "复活后解析恢复"
        );
    }

    /// 删除墓碑（SQLite 侧）：repo 软删留行；重启种子导入对墓碑行跳过
    /// 回填——config 段里仍声明的 agent 不会在重启后复活。
    #[tokio::test]
    async fn reseed_skips_tombstoned_rows() {
        use crate::sebas_state::writer::StateWriter;
        let dir = tempfile::tempdir().unwrap();
        let writer = StateWriter::start_settings(dir.path().join("settings.db")).unwrap();
        let cfg: Config = toml::from_str(
            r#"
            [acp.agents.claude]
            driver = "claude"
            path = "claude"
            "#,
        )
        .unwrap();

        seed_agents_from_config(writer.handle(), &cfg).await;
        let deleted = writer
            .handle()
            .exec(|conn| crate::sebas_state::repo::delete_agent(conn, "claude"))
            .await
            .unwrap();
        assert!(deleted, "首次删除成功");

        // 重启种子导入：墓碑行在，跳过回填。
        seed_agents_from_config(writer.handle(), &cfg).await;
        let rows = writer
            .handle()
            .exec(crate::sebas_state::repo::load_agents)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "不产生第二行");
        assert!(rows[0].is_deleted(), "墓碑行原样保留");
        assert_eq!(rows[0].source, "seed");
    }
}
