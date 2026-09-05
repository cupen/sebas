//! 6.1 端到端持久化测试（add-state-store）：mutation 提交后，即使写者
//! 进程被杀（这里用 drop writer + 重新打开模拟 SIGKILL 的持久性语义——
//! 每次 mutation 已同步提交到 SQLite，WAL 已 checkpoint），重启后状态保留。
//!
//! 真正的 SIGKILL（进程级）由 `tests/sigterm_cleanup_test.rs` 等运行时级
//! 测试覆盖；本测试聚焦「提交即持久」的 DB 契约。

use sebas::sebas_state::writer::StateWriter;
use sebas_dispatch::state_store::StateStoreEngine;

/// 写一条 settings + 一条 project → 关闭写者（模拟进程结束）→
/// 重新打开同一 DB → 数据仍在。
#[test]
fn committed_mutation_survives_writer_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("persist.db");

    // 第一次生命周期：写者 + 引擎。
    {
        let writer = StateWriter::start(path.clone()).unwrap();
        let engine = sebas::sebas_state::engine::DbStateEngine::new(writer.handle().clone());

        // settings（空对象 = 合法 CardConfig）。
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            engine
                .save_settings(serde_json::json!({}))
                .await
                .expect("save settings");
        });

        // projects: add + 读回。
        rt.block_on(async {
            engine
                .add_project("/tmp/persist-proj", "persist-proj", 1700000000)
                .await
                .expect("add project");
            let list = engine.load_projects().await.expect("load projects");
            assert_eq!(list.len(), 1, "mutation must be visible before teardown");
        });

        // 写者 drop = 进程结束（mutation 已提交，DB 已持久）。
        drop(writer);
    }

    // 第二次生命周期：重新打开同一 DB。
    {
        let writer = StateWriter::start(path.clone()).unwrap();
        let engine = sebas::sebas_state::engine::DbStateEngine::new(writer.handle().clone());
        let rt = tokio::runtime::Runtime::new().unwrap();

        // settings 在。
        rt.block_on(async {
            let settings = engine.load_settings().await.expect("load settings");
            assert!(settings.is_some(), "settings must survive restart");
        });

        // project 在。
        rt.block_on(async {
            let list = engine.load_projects().await.expect("load projects");
            assert_eq!(list.len(), 1, "project must survive restart");
            assert_eq!(list[0]["path"], "/tmp/persist-proj");
            assert_eq!(list[0]["name"], "persist-proj");
        });
    }
}

/// providers/aliases 同契约：save_persisted_state 后重启，providers + deleted
/// + model_aliases 全部保留。
#[test]
fn providers_and_aliases_survive_writer_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("persist-providers.db");
    let rt = tokio::runtime::Runtime::new().unwrap();

    // 第一次生命周期。
    {
        let writer = StateWriter::start(path.clone()).unwrap();
        let engine = sebas::sebas_state::engine::DbStateEngine::new(writer.handle().clone());
        rt.block_on(async {
            let mut state = engine.load_persisted_state().await;
            state.providers.insert(
                "anthropic".into(),
                serde_json::json!({
                    "base_url_anthropic": "https://api.anthropic.com",
                    "api_key_env": "ANTHROPIC_API_KEY",
                })
                .as_object()
                .unwrap()
                .clone(),
            );
            state.deleted.push("legacy".into());
            state.model_aliases.insert(
                "my-claude".into(),
                sebas_dispatch::state_store::ModelAliasEntry {
                    provider: "anthropic".into(),
                    upstream_model: Some("claude-sonnet-4".into()),
                },
            );
            engine
                .save_persisted_state(state.clone())
                .await
                .expect("save persisted state");
            // 立即读回验证已提交到库。
            let reloaded = engine.load_persisted_state().await;
            assert!(reloaded.providers.contains_key("anthropic"));
            assert!(reloaded.deleted.contains(&"legacy".to_string()));
            assert!(reloaded.model_aliases.contains_key("my-claude"));
        });
        drop(writer);
    }

    // 第二次生命周期。
    {
        let writer = StateWriter::start(path.clone()).unwrap();
        let engine = sebas::sebas_state::engine::DbStateEngine::new(writer.handle().clone());
        rt.block_on(async {
            let state = engine.load_persisted_state().await;
            assert!(
                state.providers.contains_key("anthropic"),
                "providers must survive restart"
            );
            assert!(
                state.deleted.contains(&"legacy".to_string()),
                "deleted tombstones must survive restart"
            );
            let alias = state
                .model_aliases
                .get("my-claude")
                .expect("alias must survive restart");
            assert_eq!(alias.provider, "anthropic");
            assert_eq!(alias.upstream_model.as_deref(), Some("claude-sonnet-4"));
        });
    }
}