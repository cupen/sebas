//! 6.1 端到端持久化测试（add-state-store）：mutation 提交后，即使写者
//! 进程被杀（这里用 drop writer + 重新打开模拟 SIGKILL 的持久性语义——
//! 每次 mutation 已同步提交到 SQLite，WAL 已 checkpoint），重启后状态保留。
//!
//! 真正的 SIGKILL（进程级）由 `tests/sigterm_cleanup_test.rs` 等运行时级
//! 测试覆盖；本测试聚焦「提交即持久」的 DB 契约。

use sebas::sebas_state::writer::StateWriter;
use sebas_dispatch::state_store::StateStoreEngine;

/// 写一条 settings + 一条 project → 关闭写者（模拟进程结束）→
/// 重新打开同一对 DB → 数据仍在。single-state-dir 起 settings 与 projects
/// 分属两库——两个写者各开一次（run.rs 的生产装配形态）。
#[test]
fn committed_mutation_survives_writer_restart() {
    let dir = tempfile::tempdir().unwrap();
    let settings_path = dir.path().join("settings.db");
    let projects_path = dir.path().join("projects.db");

    // 第一次生命周期：写者 + 引擎。
    {
        let settings = StateWriter::start(settings_path.clone()).unwrap();
        let projects = StateWriter::start_projects(projects_path.clone()).unwrap();
        let engine = sebas::sebas_state::engine::DbStateEngine::with_projects(
            settings.handle().clone(),
            projects.handle().clone(),
        );

        // settings（空对象 = 合法 CardConfig）。
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            engine
                .save_settings(serde_json::json!({}))
                .await
                .expect("save settings");
        });

        // projects: add + 读回。两条：一条本机、一条**远程节点**——远程项目
        // 必须和本机项目一样落在库里（migrate-project-registry 3.3：旧实现
        // 里远程条目只存在于 projects.json，库里根本不存在）。
        rt.block_on(async {
            engine
                .add_project("local", "/tmp/persist-proj", "persist-proj", 1700000000)
                .await
                .expect("add local project");
            engine
                .add_project("node-1", "/srv/remote-proj", "remote-proj", 1700000001)
                .await
                .expect("add remote project");
            let list = engine.load_projects().await.expect("load projects");
            assert_eq!(list.len(), 2, "mutation must be visible before teardown");
        });

        // 写者 drop = 进程结束（mutation 已提交，DB 已持久）。
        drop(settings);
        drop(projects);
    }

    // 第二次生命周期：重新打开同一对 DB。
    {
        let settings = StateWriter::start(settings_path.clone()).unwrap();
        let projects = StateWriter::start_projects(projects_path.clone()).unwrap();
        let engine = sebas::sebas_state::engine::DbStateEngine::with_projects(
            settings.handle().clone(),
            projects.handle().clone(),
        );
        let rt = tokio::runtime::Runtime::new().unwrap();

        // settings 在。
        rt.block_on(async {
            let settings = engine.load_settings().await.expect("load settings");
            assert!(settings.is_some(), "settings must survive restart");
        });

        // project 在——本机与远程都在，且节点归属原样（3.3）。
        rt.block_on(async {
            let list = engine.load_projects().await.expect("load projects");
            assert_eq!(list.len(), 2, "both projects must survive restart");
            let local = list
                .iter()
                .find(|p| p.path == "/tmp/persist-proj")
                .expect("本机项目必须存活");
            assert_eq!(local.name, "persist-proj");
            assert_eq!(local.node_id, "local");
            let remote = list
                .iter()
                .find(|p| p.path == "/srv/remote-proj")
                .expect("远程节点项目必须存活（旧实现只在文件里）");
            assert_eq!(remote.name, "remote-proj");
            assert_eq!(remote.node_id, "node-1", "节点归属必须随重启保留");
        });
    }
}

/// migrate-project-registry 4.1：**不做遗留导入**。状态目录里放一份「有远程
/// 项目」的 `projects.json`（旧实现的独家存储），库为空——打开引擎后库必须
/// 仍然为空，远程项目**不出现**在列表里（也不出现任何导入标记）：文件不再
/// 被读取，库是唯一权威。
#[test]
fn legacy_projects_file_is_not_imported() {
    let dir = tempfile::tempdir().unwrap();
    let settings_path = dir.path().join("settings.db");
    let projects_path = dir.path().join("projects.db");

    // 旧实现的落点：状态目录下的 projects.json，含一条远程项目。
    std::fs::write(
        dir.path().join("projects.json"),
        serde_json::json!({
            "version": 1,
            "projects": [{
                "id": "proj-legacyremote",
                "path": "/srv/legacy-remote",
                "name": "legacy-remote",
                "node_id": "node-legacy",
                "branch_at": 0,
                "added_at": 1,
                "sort_order": 0,
            }],
        })
        .to_string(),
    )
    .unwrap();

    let settings = StateWriter::start(settings_path).unwrap();
    let projects = StateWriter::start_projects(projects_path).unwrap();
    let engine = sebas::sebas_state::engine::DbStateEngine::with_projects(
        settings.handle().clone(),
        projects.handle().clone(),
    );
    let rt = tokio::runtime::Runtime::new().unwrap();

    let list = rt.block_on(async { engine.load_projects().await.expect("load projects") });
    assert!(
        list.is_empty(),
        "库为空时不得从 projects.json 导入任何条目: {list:?}"
    );
    // 文件仍在原地、未被改写（没有「已导入」标记，也没有被清空）。
    let raw = std::fs::read_to_string(dir.path().join("projects.json")).unwrap();
    assert!(
        raw.contains("legacy-remote"),
        "文件不得被读取方改写/清空"
    );
    assert!(
        !raw.contains("imported"),
        "不得引入任何导入标记键"
    );
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

/// env 重定向锁：`SEBAS_STATE_DIR` 是全局变量，legacy `defaults.json` 的
/// 定位由它派生，跨测试并发会撞。
///
/// retire-legacy-state-json 3.x：这里过去钉的是 `SEBAS_ROUTER_PROVIDER_OVERLAY`
/// （defaults.json 曾从 overlay 路径同目录派生）；两个 legacy 变量都已退休，
/// 现在钉状态目录。
static STATE_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 1.1 验收：经 settings 域写入默认 provider/model 后，默认值与 provider
/// 数据同库持久化（重启仍在），且不产生独立的 defaults 文件。
#[test]
fn defaults_round_trip_with_provider_data_and_no_defaults_file() {
    let _g = STATE_DIR_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: STATE_DIR_LOCK 全程持有。
    unsafe {
        std::env::set_var("SEBAS_STATE_DIR", dir.path().to_str().unwrap());
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
        assert!(
            state.providers.contains_key("deepseek"),
            "provider must survive"
        );
        assert_eq!(
            state.default_selection,
            Some(sebas_dispatch::state_store::DefaultSelection::with_model(
                "deepseek",
                "deepseek-chat"
            )),
            "defaults must survive restart alongside provider data"
        );
    });
    // 不产生独立的 defaults 文件（目录里只有那个 DB）。
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
    let _g = STATE_DIR_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let defaults = dir.path().join("defaults.json");
    std::fs::write(
        &defaults,
        r#"{"provider": "legacy", "model": "legacy-model"}"#,
    )
    .unwrap();
    // SAFETY: STATE_DIR_LOCK 全程持有。
    unsafe {
        std::env::set_var("SEBAS_STATE_DIR", dir.path().to_str().unwrap());
    }

    let path = dir.path().join("import-once.db");
    let writer = StateWriter::start(path.clone()).unwrap();
    let engine = sebas::sebas_state::engine::DbStateEngine::new(writer.handle().clone());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // 第一次启动：导入。
        assert!(
            sebas::sebas_state::defaults_import::import_legacy_defaults_once(writer.handle())
                .await
                .unwrap()
        );
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
        assert!(
            !sebas::sebas_state::defaults_import::import_legacy_defaults_once(writer.handle())
                .await
                .unwrap()
        );
        let state = engine.load_persisted_state().await;
        assert_eq!(
            state.default_selection,
            Some(sebas_dispatch::state_store::DefaultSelection::new(
                "newpick"
            )),
            "标记在场后 legacy 文件不得覆盖库内选择"
        );
    });
}

// ---- session_checkpoints 表（fix-webui-qa-round9 1.1，
// session-transcript-durability）----

/// checkpoint 行经 projects 库引擎的写入/回放/删除全链路：save 即提交
/// （响应返回前库中可见）、同键 upsert 原子替换（一会话一行）、delete 幂等；
/// 写者重启（模拟强杀）后快照保留——「提交即持久」的 DB 契约。表由注册表
/// 启动同步原地在旧库上补建（新表无破坏步骤）。
#[test]
fn session_checkpoint_rows_round_trip_through_the_projects_engine() {
    use sebas_models::checkpoint::SessionCheckpointRow;

    let dir = tempfile::tempdir().unwrap();
    let settings_path = dir.path().join("settings.db");
    let projects_path = dir.path().join("projects.db");

    fn row(sid: &str, usage_in: i64) -> SessionCheckpointRow {
        SessionCheckpointRow {
            session_id: sid.into(),
            updated_at: 1_700_000_000,
            transcript_json: format!(r#"[{{"position":0,"session_id":"{sid}"}}]"#),
            parked_json: r#"[{"request_id":"claude:tc-1","tool_name":"Bash","args":{}}]"#.into(),
            usage_in,
            usage_out: 25,
            usage_reported: true,
        }
    }

    let rt = tokio::runtime::Runtime::new().unwrap();

    // 第一次生命周期：写两行 + 覆盖其中一行。
    {
        let settings = StateWriter::start_settings(settings_path.clone()).unwrap();
        let projects = StateWriter::start_projects(projects_path.clone()).unwrap();
        let engine = sebas::sebas_state::engine::DbStateEngine::with_projects(
            settings.handle().clone(),
            projects.handle().clone(),
        );
        rt.block_on(async {
            engine
                .save_session_checkpoint(row("sess-1", 10))
                .await
                .expect("save checkpoint 1");
            engine
                .save_session_checkpoint(row("sess-2", 0))
                .await
                .expect("save checkpoint 2");
            // 同键覆盖：不产生第二行（单会话单行快照）。
            engine
                .save_session_checkpoint(row("sess-1", 99))
                .await
                .expect("overwrite checkpoint 1");

            let mut rows = engine.load_session_checkpoints().await.expect("load");
            rows.sort_by(|a, b| a.session_id.cmp(&b.session_id));
            assert_eq!(rows.len(), 2, "upsert must replace, not duplicate");
            assert_eq!(rows[0].session_id, "sess-1");
            assert_eq!(rows[0].usage_in, 99, "latest snapshot wins");
            assert_eq!(rows[0].usage_reported, true);
            assert_eq!(rows[1].session_id, "sess-2");
        });

        // 删除一行：close 归档路径的存储半边。
        rt.block_on(async {
            engine
                .delete_session_checkpoint("sess-2")
                .await
                .expect("delete checkpoint");
            let rows = engine.load_session_checkpoints().await.expect("load");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].session_id, "sess-1");
            // 幂等：再删同键不报错。
            engine
                .delete_session_checkpoint("sess-2")
                .await
                .expect("idempotent delete");
        });
        // 写者 drop = 进程结束（mutation 已同步提交）。
        drop(settings);
        drop(projects);
    }

    // 第二次生命周期：重新打开同一库——未删的快照在（强杀后回放的数据源）。
    {
        let settings = StateWriter::start_settings(settings_path.clone()).unwrap();
        let projects = StateWriter::start_projects(projects_path.clone()).unwrap();
        let engine = sebas::sebas_state::engine::DbStateEngine::with_projects(
            settings.handle().clone(),
            projects.handle().clone(),
        );
        rt.block_on(async {
            let rows = engine.load_session_checkpoints().await.expect("load");
            assert_eq!(rows.len(), 1, "checkpoint must survive writer restart");
            assert_eq!(rows[0].session_id, "sess-1");
            assert_eq!(rows[0].usage_in, 99);
            assert!(rows[0].transcript_json.contains("sess-1"));
        });
    }
}

/// 不可用的 projects 库：checkpoint 域方法如实拒绝（不拿默认 no-op 冒充
/// 成功）。
#[test]
fn unavailable_projects_db_rejects_checkpoint_writes_honestly() {
    let dir = tempfile::tempdir().unwrap();
    let settings = StateWriter::start_settings(dir.path().join("settings.db")).unwrap();
    let blocked = dir.path().join("projects.db");
    std::fs::create_dir_all(&blocked).unwrap();
    let cause = StateWriter::start_projects(blocked.clone())
        .err()
        .expect("目录路径上 open 必失败")
        .trim()
        .to_string();
    let engine = sebas::sebas_state::engine::DbStateEngine::with_unavailable_projects(
        settings.handle().clone(),
        cause,
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let err = engine
            .save_session_checkpoint(sebas_models::checkpoint::SessionCheckpointRow {
                session_id: "s".into(),
                updated_at: 0,
                transcript_json: "[]".into(),
                parked_json: "[]".into(),
                usage_in: 0,
                usage_out: 0,
                usage_reported: false,
            })
            .await
            .unwrap_err();
        assert!(err.contains("不可用"), "{err}");
        assert!(engine.load_session_checkpoints().await.is_err());
        assert!(engine.delete_session_checkpoint("s").await.is_err());
    });
}

/// fix-webui-qa-round9 3.3（project-session-actions「rename via rail menu」的
/// 持久半边，review 阶段补写）：rename 走端点同一条 mutation 路径落库后，
/// 写者重启（模拟 core 重启）名字保留；且 UPDATE 只碰 name 列——path、
/// 注册时刻、排序、节点归属、项目级默认 agent 一概原样（spec「Renaming
/// SHALL NOT change the project's path, sort order, node attribution」；
/// 会话从属由 path 派生，path 不动即从属不动）。未知 id 如实报「不存在」，
/// 不假装成功。
#[test]
fn project_rename_survives_writer_restart_and_moves_only_the_name() {
    let dir = tempfile::tempdir().unwrap();
    let settings_path = dir.path().join("settings.db");
    let projects_path = dir.path().join("projects.db");

    let id = {
        let settings = StateWriter::start_settings(settings_path.clone()).unwrap();
        let projects = StateWriter::start_projects(projects_path.clone()).unwrap();
        let engine = sebas::sebas_state::engine::DbStateEngine::with_projects(
            settings.handle().clone(),
            projects.handle().clone(),
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            engine
                .add_project("local", "/tmp/rename-proj", "rename-proj", 1_700_000_000)
                .await
                .expect("add project");
            engine
                .set_project_default_agent(
                    &sebas_models::project::project_id_for_on("local", "/tmp/rename-proj"),
                    "claude",
                )
                .await
                .expect("set default agent");
            let before = engine
                .load_projects()
                .await
                .expect("load before rename")
                .into_iter()
                .next()
                .expect("one project row");
            let id = before.id.clone().expect("registered row carries its id");

            // 端点同一条 mutation 路径（api.rs projects_rename → projects
            // op rename）：trim 在 handler/mutation 层完成，落库的是 trim 后
            // 的名字（engine 层如实存储，不重复 trim）。
            engine
                .rename_project(&id, "新名字")
                .await
                .expect("rename committed");
            id
        })
        // 写者 drop = core 进程结束。
    };

    // 第二次生命周期：重启后名字保留、其余列原样。
    let settings = StateWriter::start_settings(settings_path.clone()).unwrap();
    let projects = StateWriter::start_projects(projects_path.clone()).unwrap();
    let engine = sebas::sebas_state::engine::DbStateEngine::with_projects(
        settings.handle().clone(),
        projects.handle().clone(),
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let list = engine.load_projects().await.expect("load after restart");
        assert_eq!(list.len(), 1);
        let row = &list[0];
        assert_eq!(row.id.as_deref(), Some(id.as_str()), "stable id unchanged");
        assert_eq!(row.name, "新名字", "renamed name survives the restart");
        assert_eq!(row.path, "/tmp/rename-proj", "path must not move");
        assert_eq!(row.node_id, "local", "node attribution must not move");
        assert_eq!(row.added_at, 1_700_000_000, "added_at must not move");
        assert_eq!(row.sort_order, 0, "sort order must not move");
        assert_eq!(
            row.default_agent.as_deref(),
            Some("claude"),
            "project default agent must not move"
        );

        // 未知 id → Err「不存在」（DbStateEngine 返回 false，mutation 层转
        // 404）——绝不假装成功。
        let missing = engine.rename_project("proj-doesnotexist", "x").await;
        assert!(
            matches!(&missing, Ok(false)),
            "unknown id must report no-row-updated: {missing:?}"
        );
    });
}
