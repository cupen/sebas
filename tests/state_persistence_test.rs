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

// ---- make-core-own-provider-data 1.1/1.4：defaults 并入 settings 域 ----

/// env 重定向锁：SEBAS_ROUTER_PROVIDER_OVERLAY 是全局变量，defaults.json
/// 的定位派生自它，跨测试并发会撞。
static OVERLAY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 1.1 验收：经 settings 域写入默认 provider/model 后，默认值与 provider
/// 数据同库持久化（重启仍在），且不产生独立的 defaults 文件。
#[test]
fn defaults_round_trip_with_provider_data_and_no_defaults_file() {
    let _g = OVERLAY_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let overlay = dir.path().join("providers.json");
    std::fs::write(&overlay, r#"{"providers": {}, "deleted": []}"#).unwrap();
    // SAFETY: OVERLAY_LOCK 全程持有。
    unsafe {
        std::env::set_var(
            "SEBAS_ROUTER_PROVIDER_OVERLAY",
            overlay.to_str().unwrap(),
        );
    }

    let path = dir.path().join("defaults-domain.db");
    let writer = StateWriter::start(path.clone()).unwrap();
    let engine = sebas::sebas_state::engine::DbStateEngine::new(writer.handle().clone());
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        // provider + defaults 同域提交（providers 域 put + settings 域
        // set_defaults，两次 RMW 各自整事务落库）。
        let mut state = engine.load_persisted_state().await;
        state.providers.insert(
            "deepseek".into(),
            serde_json::json!({"preset": "deepseek", "api_key": "sk-x"})
                .as_object()
                .unwrap()
                .clone(),
        );
        engine
            .save_persisted_state(state)
            .await
            .expect("save provider");
        sebas_dispatch::state_store::settings_mutation(
            &engine,
            &serde_json::json!({"op": "set_defaults", "provider": "deepseek", "model": "deepseek-chat"}),
        )
        .await
        .expect("set defaults via settings domain");
    });
    drop(writer);

    // 重启：provider 数据与默认值都还在。
    let writer = StateWriter::start(path).unwrap();
    let engine = sebas::sebas_state::engine::DbStateEngine::new(writer.handle().clone());
    rt.block_on(async {
        let state = engine.load_persisted_state().await;
        assert!(state.providers.contains_key("deepseek"), "provider must survive");
        assert_eq!(
            state.default_selection,
            Some(sebas_dispatch::state_store::DefaultSelection::with_model(
                "deepseek",
                "deepseek-chat"
            )),
            "defaults must survive restart alongside provider data"
        );
    });
    // 不产生独立的 defaults 文件（目录里只有我们预置的 providers.json 与 DB）。
    let produced: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("defaults.json"))
        .collect();
    assert!(
        produced.is_empty(),
        "defaults 并入库后不得再产生 defaults 文件: {produced:?}"
    );
}

/// 1.4 验收：有 defaults.json 时导入一次；再次启动不重复导入（库里的
/// 后续变化不被 legacy 文件覆盖）。
#[test]
fn legacy_defaults_json_imports_exactly_once() {
    let _g = OVERLAY_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let overlay = dir.path().join("providers.json");
    std::fs::write(&overlay, r#"{"providers": {}, "deleted": []}"#).unwrap();
    let defaults = dir.path().join("defaults.json");
    std::fs::write(&defaults, r#"{"provider": "legacy", "model": "legacy-model"}"#).unwrap();
    // SAFETY: OVERLAY_LOCK 全程持有。
    unsafe {
        std::env::set_var(
            "SEBAS_ROUTER_PROVIDER_OVERLAY",
            overlay.to_str().unwrap(),
        );
    }

    let path = dir.path().join("import-once.db");
    let writer = StateWriter::start(path.clone()).unwrap();
    let engine = sebas::sebas_state::engine::DbStateEngine::new(writer.handle().clone());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // 第一次启动：导入。
        assert!(sebas::sebas_state::defaults_import::import_legacy_defaults_once(writer.handle())
            .await
            .unwrap());
        let state = engine.load_persisted_state().await;
        assert_eq!(
            state.default_selection,
            Some(sebas_dispatch::state_store::DefaultSelection::with_model(
                "legacy",
                "legacy-model"
            ))
        );

        // 用户改了默认（库内新值）。
        sebas_dispatch::state_store::settings_mutation(
            &engine,
            &serde_json::json!({"op": "set_defaults", "provider": "newpick"}),
        )
        .await
        .unwrap();

        // legacy 文件被改写（模拟旧二进制又写了一次）→ 再次启动不得回灌。
        std::fs::write(&defaults, r#"{"provider": "rewritten", "model": null}"#).unwrap();
        assert!(!sebas::sebas_state::defaults_import::import_legacy_defaults_once(
            writer.handle()
        )
        .await
        .unwrap());
        let state = engine.load_persisted_state().await;
        assert_eq!(
            state.default_selection,
            Some(sebas_dispatch::state_store::DefaultSelection::new("newpick")),
            "标记在场后 legacy 文件不得覆盖库内选择"
        );
    });
}