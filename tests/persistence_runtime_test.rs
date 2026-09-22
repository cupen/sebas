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
