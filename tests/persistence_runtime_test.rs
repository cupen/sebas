//! extract-sebas-db 1.3：`sebas-db` 叶子属性与域无关性的机械断言。
//!
//! spec「The execution model carries no domain knowledge」与 design D1 的
//! 依赖图红线，在这里以机器可核对的方式钉住：
//!
//! 1. `cargo tree -p sebas-db` 不得出现任何 sebas-* crate（叶子属性——
//!    引入域依赖或角色 crate 即失败）；
//! 2. `sebas-db` 的公开源文件不得出现域表名字面量或域/角色 crate 引用
//!    （公开面纪律——域表名 `projects` / `session_map` / `users` 等只能
//!    出现在根注册表或按写入者归属的模型 crate）。

use std::path::{Path, PathBuf};
use std::process::Command;

const DOMAIN_TABLE_LITERALS: &[&str] = &[
    // 带引号的字符串字面量形态：正常代码文本不会包含这些带引号的域表名。
    "\"projects\"",
    "\"session_map\"",
    "\"model_aliases\"",
    "\"providers\"",
    "\"settings\"",
    "\"users\"",
];

const FORBIDDEN_CRATE_REFS: &[&str] = &[
    "sebas_domain",
    "sebas_dispatch",
    "sebas_webui",
    "sebas_router",
    "sebas_im",
    "sebas_feishu",
    "sebas_acp",
    "sebas_channels",
    "sebas_node",
    "sebas_models",
    "sebas_state",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn sebas_db_src_dir() -> PathBuf {
    workspace_root().join("sebas-db").join("src")
}

fn rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries = std::fs::read_dir(&d).expect("read dir");
        while let Some(entry) = entries.next().transpose().expect("dir entry") {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// 1.3 验收（依赖图）：sebas-db 的依赖树不得包含任何 sebas-* crate。
/// 临时给 sebas-db 加一个域依赖（如 `sebas-domain = { path = "../sebas-domain" }`）
/// 本测试即红（失败演示已做并在 change 汇报中记录）。
#[test]
fn sebas_db_dependency_tree_contains_no_sebas_crates() {
    let output = Command::new(env!("CARGO"))
        .args(["tree", "-p", "sebas-db"])
        .current_dir(workspace_root())
        .output()
        .expect("cargo tree must run");
    assert!(
        output.status.success(),
        "cargo tree -p sebas-db failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let tree = String::from_utf8_lossy(&output.stdout);
    let mut lines = tree.lines();
    let root = lines.next().expect("tree prints the package itself");
    assert!(
        root.starts_with("sebas-db"),
        "tree root should be sebas-db itself: {root}"
    );
    for line in lines {
        assert!(
            !line.contains("sebas-"),
            "sebas-db 依赖树出现 sebas-* crate（叶子属性被破坏）: {line}"
        );
    }
}

/// 1.3 验收（公开面）：sebas-db 源文件不得出现域表名字面量或域/角色
/// crate 引用——runtime 只认识泛型 Record 与元数据，不认识任何域表。
#[test]
fn sebas_db_public_surface_names_no_domain_tables_or_role_crates() {
    let files = rs_files(&sebas_db_src_dir());
    assert!(
        !files.is_empty(),
        "sebas-db/src must contain rust sources"
    );

    for file in files {
        let rel = file.strip_prefix(workspace_root()).unwrap();
        let src = std::fs::read_to_string(&file).expect("read source");
        for table in DOMAIN_TABLE_LITERALS {
            assert!(
                !src.contains(table),
                "{} 出现域表名字面量 {table}（域 schema 事实不得下沉 sebas-db）",
                rel.display()
            );
        }
        for crate_ref in FORBIDDEN_CRATE_REFS {
            assert!(
                !src.contains(crate_ref),
                "{} 引用了域/角色 crate `{crate_ref}`（sebas-db 必须域无关）",
                rel.display()
            );
        }
    }
}

/// derive 生成路径已迁移（design D3）：sebas-schema-derive 不再生成指向根
/// crate 的路径，任何 crate 声明 schema 模型只需依赖 sebas-db。
#[test]
fn schema_derive_references_shared_persistence_crate_only() {
    let derive_lib =
        std::fs::read_to_string(workspace_root().join("sebas-schema-derive").join("src").join("lib.rs"))
            .expect("read derive lib.rs");
    assert!(
        !derive_lib.contains("sebas_state"),
        "derive 生成路径不得再指向根 crate 的 sebas_state"
    );
    assert!(
        derive_lib.contains(":: sebas_db :: schema :: SchemaColumn"),
        "derive 生成路径应指向 ::sebas_db::schema::SchemaColumn"
    );
}

/// spec「A second database does not re-implement the recipe」（review 补钉，
/// 原 5.5 是一次性 grep 验收）：sebas-webui 里不得再出现本地 pragma 组合或
/// 自选事务行为——连接取 `sebas_db::conn::open`，Immediate 事务取共享入口
/// （design D5：共享层两种事务都提供，调用方的选择经共享入口表达）。
#[test]
fn webui_user_store_reuses_shared_recipe_instead_of_local_copy() {
    for file in rs_files(&workspace_root().join("sebas-webui").join("src")) {
        let rel = file.strip_prefix(workspace_root()).unwrap();
        let src = std::fs::read_to_string(&file).expect("read source");
        for forbidden in ["pragma_update", "transaction_with_behavior"] {
            assert!(
                !src.contains(forbidden),
                "{} 出现本地配方 `{forbidden}`（auth.db 必须经 sebas-db 拿连接与事务入口）",
                rel.display()
            );
        }
    }
    let user_store =
        std::fs::read_to_string(workspace_root().join("sebas-webui").join("src").join("user_store.rs"))
            .expect("read user_store.rs");
    assert!(
        user_store.contains("sebas_db::conn::open"),
        "user_store 应从共享层取连接（sebas_db::conn::open）"
    );
    assert!(
        user_store.contains("sebas_db::conn::transaction_immediate"),
        "user_store 的 Immediate 事务应走共享入口（design D5）"
    );
}

/// spec「No component carries a private migration mechanism」（review 补钉）：
/// 表结构 diff 的实现（`pragma_table_info` 的使用）只允许存在于 sebas-db——
/// 原已删除的 `src/sebas_state/migration.rs` 不得回魂，其他 crate 也不得长出
/// 第二份 table-diffing。
#[test]
fn table_diffing_exists_only_in_sebas_db() {
    assert!(
        !workspace_root().join("src").join("sebas_state").join("migration.rs").exists(),
        "src/sebas_state/migration.rs 已下沉 sebas-db，不得回魂"
    );
    let mut scanned = 0;
    for dir in [
        workspace_root().join("src"),
        workspace_root().join("sebas-webui").join("src"),
        workspace_root().join("sebas-models").join("src"),
        workspace_root().join("sebas-router").join("src"),
    ] {
        for file in rs_files(&dir) {
            let rel = file.strip_prefix(workspace_root()).unwrap();
            let src = std::fs::read_to_string(&file).expect("read source");
            scanned += 1;
            assert!(
                !src.contains("pragma_table_info"),
                "{} 出现表结构 diff 原语 `pragma_table_info`（迁移机制只允许 sebas-db 一份）",
                rel.display()
            );
        }
    }
    assert!(scanned > 0, "至少应扫到根 crate 与三个角色 crate 的源码");
}

/// persist-router-usage 2.3（静态面）：router 的源码里不得出现 core 状态库的
/// 逻辑名或其覆盖变量。`state-store`「Only the core process SHALL open the
/// database」——router 只开自己的 `usage.db`，settings.db / projects.db
/// 连**名字**都不该出现（出现即意味着有人把 core 的库接进了 router）。
///
/// 说明：core 的库在源码里以 `sebas_domain::state_paths` 的逻辑名
/// （`Database::Settings` / `Database::Projects` / `StatePath::SettingsDb` /
/// `StatePath::ProjectsDb`）表达，不是字符串字面量——所以这里扫的是那些
/// **标识符**，而不是 `"settings.db"` 这种 grep 不到的字面量。
///
/// 扫描**剥掉注释行**：文档注释里合法地写着「router 绝不打开 settings.db /
/// projects.db」这类禁令说明，本断言针对的是**代码**引用。
#[test]
fn router_never_references_the_core_state_databases() {
    const FORBIDDEN: &[&str] = &[
        // 逻辑名（枚举变体形态）
        "Database::Settings",
        "Database::Projects",
        "StatePath::SettingsDb",
        "StatePath::ProjectsDb",
        // 落点覆盖变量与库文件名（任何形态的硬编码都不得出现）
        "SEBAS_SETTINGS_DB",
        "SEBAS_PROJECTS_DB",
        "settings.db",
        "projects.db",
    ];
    // 逐行剥注释：`//` 起始之后的内容不参与匹配（含 `//!` / `///`）。
    let code_only = |src: &str| -> String {
        src.lines()
            .map(|line| match line.find("//") {
                Some(i) => &line[..i],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let mut scanned = 0;
    for dir in [
        workspace_root().join("sebas-router").join("src"),
        workspace_root().join("sebas-router").join("tests"),
    ] {
        for file in rs_files(&dir) {
            let rel = file.strip_prefix(workspace_root()).unwrap();
            let src = std::fs::read_to_string(&file).expect("read source");
            let src = code_only(&src);
            scanned += 1;
            for needle in FORBIDDEN {
                assert!(
                    !src.contains(needle),
                    "{} 的**代码**里出现 core 状态库的引用 `{needle}`：router 只能打开自己的 usage.db",
                    rel.display()
                );
            }
        }
    }
    assert!(scanned > 0, "至少应扫到 router 的 src 与 tests");
}

/// persist-router-usage 2.3（动态面）：router 写完用量后，core 状态库的
/// 字节与 mtime **逐项未变**——router 只开 `usage.db`，不 touch 两个 core 库。
///
/// 与静态断言互补：静态面挡「写进源码」，这条挡「真的碰了文件」。
#[tokio::test]
async fn router_usage_write_touches_only_its_own_database() {
    use sebas_router::usage::{RetentionPolicy, UsageRecord, UsageSink};

    let dir = tempfile::tempdir().expect("tempdir");
    // 三个库并列放在同一个目录里，模拟状态目录的真实布局。
    let usage = dir.path().join("usage.db");
    let settings = dir.path().join("settings.db");
    let projects = dir.path().join("projects.db");

    // core 的两个库：预置一点内容（各自独立的库文件，非 WAL 之外的形式）。
    for (path, table) in [(&settings, "settings"), (&projects, "projects")] {
        let conn = sebas_db::conn::open(path).expect("open core db");
        conn.execute_batch(&format!("create table {table} (k TEXT PRIMARY KEY, v TEXT);"))
            .expect("ddl");
        conn.execute(
            &format!("insert into {table} (k, v) values ('seed', 'untouched')"),
            [],
        )
        .expect("seed");
    }
    let snapshot = |p: &std::path::Path| {
        (
            std::fs::read(p).expect("read core db"),
            std::fs::metadata(p).unwrap().modified().unwrap(),
        )
    };
    let settings_before = snapshot(&settings);
    let projects_before = snapshot(&projects);

    // router 侧：写用量记录（只经 usage_db 那一个路径）。
    let sink = UsageSink::spawn_writer(
        &usage,
        RetentionPolicy {
            prune_interval_secs: 0,
            ..Default::default()
        },
    )
    .expect("spawn_writer");
    sink.record(UsageRecord {
        ts: "2026-08-07T00:00:00+00:00".into(),
        key: String::new(),
        protocol: "anthropic".into(),
        model: Some("m".into()),
        provider: "p".into(),
        upstream_model: None,
        status: 200,
        latency_ms: 1,
        ttft_ms: None,
        input_tokens: Some(1),
        output_tokens: Some(2),
        cache_read_tokens: None,
        cache_creation_tokens: None,
        error: None,
    });
    drop(sink);
    // 等后台 writer 落账（轮询 usage.db 出现记录即可）。
    let mut landed = false;
    for _ in 0..100 {
        if let Ok(conn) = sebas_db::conn::open_readonly(&usage)
            && let Ok(n) = conn.query_row("SELECT COUNT(*) FROM usage_records", [], |r| {
                r.get::<_, i64>(0)
            })
            && n >= 1
        {
            landed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(landed, "用量记录必须落进 router 自己的 usage.db");

    // core 的两个库：字节与 mtime 逐项未变。
    assert_eq!(
        snapshot(&settings),
        settings_before,
        "router 写用量不得改动 settings.db"
    );
    assert_eq!(
        snapshot(&projects),
        projects_before,
        "router 写用量不得改动 projects.db"
    );
    // 用量库确实被创建/写入（正向对照，避免断言因路径打错而空过）。
    assert!(usage.exists(), "usage.db 必须被创建");
}
