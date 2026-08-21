//! 集成测试：验收 `upgrade dev`（DEV 升级）的每个阶段与升级结果。
//!
//! 直接 in-process 调用 `sebas::update::run(UpdateArgs)`，复用 `tests/support` 的
//! `TestDir` 做 scratch，把 `[watchdog.storage] data_dir` 指向一个临时目录以完全
//! 隔离安装落盘（versions/、current、rollback、upgrade.lock）。
//!
//! 真实安装路径用一个**合成 sebas crate**（`src/main.rs` 打印一行 + 空依赖），
//! 让 `compile_dev` 触发 `cargo build --release` 数秒完成，而不是真仓库 ~95s。
//! 验收的是「落盘结果与阶段序」；二进制内容本身非关注点。
//!
//! 阶段链（`updater.rs` / `upgrade.rs`，无具名阶段机，阶段隐含在调用序中）：
//!   入口/plan → dry-run 只规划不落盘 → 锁(`try_lock`) → compile_dev →
//!   install_version（versions/vdev-* / current 软链 / rollback 备份）→ 解锁 → 回滚。
//!
//! 默认跑：dry-run 与项目校验（不真实编译、不落盘安装）。
//! `#[ignore]`：真实 install → 重装 → rollback 完整链（需本机 cargo；合成项目秒级）。
//!
//! ⚠️ `upgrade::try_lock` 用进程内全局静态锁 `UPGRADE_LOCKED`（`upgrade.rs`），且
//! `run_one_shot_with_config` 对 dry-run / rollback / dev 一律 `try_lock`/`unlock`。
//! 同一测试二进制内并行用例会撞锁("正在升级中")。因此**所有调用 `run` 的用例都经
//! 全局 `SERIAL` 互斥锁串行化**。

mod support;

use sebas::update::{UpdateArgs, run};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// 串行化所有会触碰进程内全局 `UPGRADE_LOCKED` 的 `run` 调用。
static SERIAL: OnceLock<Mutex<()>> = OnceLock::new();

async fn run_serial(args: UpdateArgs) -> Result<(), String> {
    let _guard = SERIAL.get_or_init(|| Mutex::new(())).lock().unwrap();
    run(args).await.map_err(|e| e.to_string())
}

/// 在 `dir` 下写一个最小 sebas 配置：一个假 Feishu credential 满足
/// `Config::validate`，`[watchdog.storage] data_dir` 指向 `data_dir`。
fn write_config(dir: &Path, data_dir: &Path) -> PathBuf {
    // `try_lock` 直接在 `data_dir/upgrade.lock` 上建文件；data_dir 必须先存在，
    // 否则锁创建会以 "创建锁文件失败" 告败。真实安装目录在首次安装前已存在，
    // 这里显式建好以对齐。
    std::fs::create_dir_all(data_dir).unwrap();
    let toml = format!(
        "[feishu]\napp_id = \"test-app\"\napp_secret = \"test-secret\"\n\n\
         [watchdog.storage]\ndata_dir = \"{}\"\n",
        data_dir.display()
    );
    let path = dir.join("config.toml");
    std::fs::write(&path, toml).unwrap();
    path
}

/// 在 `dir` 下生成一个可被 `compile_dev` 接受、且能秒级 `cargo build --release`
/// 的合成 sebas crate。返回 crate 根目录。
fn write_synthetic_project(dir: &Path, package_name: &str) -> PathBuf {
    let project = dir.join("fixture-crate");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [workspace]\n"
        ),
    )
    .unwrap();
    std::fs::write(
        project.join("src").join("main.rs"),
        "fn main() { println!(\"synthetic sebas\"); }\n",
    )
    .unwrap();
    project
}

// ── B. dry-run 阶段：只规划、不落盘 ──────────────────────────────
#[tokio::test]
async fn dev_dry_run_validates_project_but_does_not_install() {
    let work = support::TestDir::new("upgrade_dev", "dry-ok");
    let data_dir = work.path().join("data");
    let cfg = write_config(work.path(), &data_dir);
    let crate_dir = write_synthetic_project(work.path(), "sebas");

    run_serial(UpdateArgs {
        dev: true,
        dry_run: true,
        rollback: false,
        project_dir: Some(crate_dir),
        config: cfg.to_str().unwrap().into(),
    })
    .await
    .expect("dry-run dev 应返回 Ok");

    // dry-run 只规划、不落盘安装。
    assert!(!data_dir.join("current").exists(), "dry-run 不应建 current");
    assert!(
        !data_dir.join("versions").exists(),
        "dry-run 不应建 versions/"
    );
    assert!(
        !data_dir.join("upgrade.lock").exists(),
        "dry-run 结束后不应残留 upgrade.lock"
    );
}

#[tokio::test]
async fn dry_run_rejects_project_without_cargo_toml() {
    let work = support::TestDir::new("upgrade_dev", "dry-no-proj");
    let data_dir = work.path().join("data");
    let cfg = write_config(work.path(), &data_dir);
    // 指向一个没有 Cargo.toml 的目录 → dry-run 校验应失败。
    let not_a_crate = work.path().join("empty-dir");
    std::fs::create_dir_all(&not_a_crate).unwrap();

    let err = run_serial(UpdateArgs {
        dev: true,
        dry_run: true,
        rollback: false,
        project_dir: Some(not_a_crate),
        config: cfg.to_str().unwrap().into(),
    })
    .await
    .expect_err("缺 Cargo.toml 的 dry-run dev 应报错");
    assert!(
        err.contains("不是 Rust 项目目录") || err.contains("Cargo.toml"),
        "缺 Cargo.toml 时应报项目校验错误，实际: {err}"
    );
    assert!(
        !data_dir.join("current").exists(),
        "失败的 dry-run 也不应落盘 current"
    );
}

// ── C. 真实安装链（合成 crate，秒级；#[ignore]）────────────────
/// 单个串行用例走完 安装 → 重装 → 回滚，避免多个真实用例在全局
/// `UPGRADE_LOCKED` 上撞锁。逐阶段断言落盘结构。
#[tokio::test]
#[ignore]
async fn real_dev_install_then_reinstall_then_rollback() {
    let work = support::TestDir::new("upgrade_dev", "real-chain");
    let data_dir = work.path().join("data");
    let cfg = write_config(work.path(), &data_dir);
    let crate_dir = write_synthetic_project(work.path(), "sebas");

    // 1) 首次真实安装：compile_dev + install_version 落地。
    run_serial(UpdateArgs {
        dev: true,
        dry_run: false,
        rollback: false,
        project_dir: Some(crate_dir.clone()),
        config: cfg.to_str().unwrap().into(),
    })
    .await
    .expect("首次 dev 安装应成功");

    let current = data_dir.join("current");
    let version_dir = data_dir.join("versions").join("vdev-0.1.0"); // current_version_raw()=="0.1.0"
    assert!(current.exists(), "current 软链应存在");
    assert!(
        std::fs::symlink_metadata(&current)
            .unwrap()
            .file_type()
            .is_symlink(),
        "current 应为软链"
    );
    let link_target = std::fs::read_link(&current).unwrap();
    assert_eq!(
        link_target,
        Path::new("versions/vdev-0.1.0"),
        "current 应指向刚装的版本目录，实际: {link_target:?}"
    );

    let installed = version_dir.join("sebas");
    assert!(installed.exists(), "versions/vdev-0.1.0/sebas 应存在");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&installed).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "已安装二进制应可执行，mode={mode:o}");
    }

    // 版本目录下只有 sebas，无多余文件。
    let version_entries: Vec<_> = std::fs::read_dir(&version_dir)
        .unwrap()
        .map(|e| e.map(|e| e.file_name()).unwrap())
        .collect();
    assert_eq!(version_entries.len(), 1, "版本目录应只有 sebas");

    // 编译产物被复制进版本目录：源==副本。
    let compiled = crate_dir.join("target/release/sebas");
    assert!(compiled.exists(), "compile_dev 应产出 target/release/sebas");
    assert_eq!(
        std::fs::read(&installed).unwrap(),
        std::fs::read(&compiled).unwrap(),
        "安装副本应与编译产物一致"
    );
    assert!(!data_dir.join("upgrade.lock").exists(), "结束后应无锁残留");

    // 2) 重装同版本：versions 不变，rollback/ 出现备份（= 首装）。
    run_serial(UpdateArgs {
        dev: true,
        dry_run: false,
        rollback: false,
        project_dir: Some(crate_dir.clone()),
        config: cfg.to_str().unwrap().into(),
    })
    .await
    .expect("重装应成功");

    let rollback_bin = data_dir.join("rollback/sebas");
    assert!(rollback_bin.exists(), "重装后应备份旧版到 rollback/");
    assert_eq!(
        std::fs::read(&rollback_bin).unwrap(),
        std::fs::read(&installed).unwrap(),
        "rollback 备份应与首装一致"
    );

    // 3) 回滚至上一版本：current → versions/rollback。
    run_serial(UpdateArgs {
        dev: false,
        dry_run: false,
        rollback: true,
        project_dir: None,
        config: cfg.to_str().unwrap().into(),
    })
    .await
    .expect("回滚应成功");

    let target = std::fs::read_link(&current).unwrap();
    assert_eq!(
        target,
        Path::new("versions/rollback"),
        "回滚后 current 应指 versions/rollback，实际: {target:?}"
    );
    assert!(
        data_dir.join("versions/rollback/sebas").exists(),
        "回滚版本目录应有二进制"
    );
}
