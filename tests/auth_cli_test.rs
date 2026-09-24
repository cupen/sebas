//! `sebas auth` CLI 集成测试（add-auth-subcommand 3.1；auth-cli spec 场景）：
//! add 成功/重名（大小写不敏感）、passwd 改密/缺户/拒 --role、list 含字段
//! 无哈希、`SEBAS_WEBUI_AUTH_DB` env 覆盖路径、明文不落盘扫描、短密码告警
//! 不拦截、来源互斥、缺密码、空密码、退役 `webui-passwd` 未知子命令。
//!
//! 全部经真二进制（`CARGO_BIN_EXE_sebas`）子进程驱动，`SEBAS_WEBUI_AUTH_DB`
//! 逐子进程显式钉进 tempdir 沙箱（Command::env，不污染测试进程全局 env），
//! 绝不触真实 `~/.sebas`。「改密对运行中 webui 即时生效」「开关关闭仍可
//! 管理」两个场景需要起 webui，归验收套件（tasks 4.2）回归。

use sebas_webui::user_store::UserStore;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// 沙箱：一个 tempdir 承载 auth.db。
struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        Sandbox {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn db(&self) -> PathBuf {
        self.dir.path().join("auth.db")
    }
}

/// 跑一次 sebas 子命令：`SEBAS_WEBUI_AUTH_DB` 只注入该子进程；`stdin`
/// 为 Some 时以管道喂入（`--password-stdin` 路径）。
fn run_sebas(db: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sebas"));
    cmd.args(args).env("SEBAS_WEBUI_AUTH_DB", db);
    match stdin {
        Some(input) => {
            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = cmd.spawn().expect("spawn sebas");
            child
                .stdin
                .take()
                .expect("piped stdin")
                .write_all(input.as_bytes())
                .expect("write password to stdin");
            child.wait_with_output().expect("wait sebas")
        }
        None => cmd.output().expect("run sebas"),
    }
}

fn auth(db: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut full: Vec<&str> = vec!["auth"];
    full.extend_from_slice(args);
    run_sebas(db, &full, stdin)
}

fn open_store(db: &Path) -> UserStore {
    // 打开既有库：迭代数默认值不影响已存用户（逐行带各自的 iterations）。
    UserStore::open_with_iterations(db, 1000).expect("open sandbox store")
}

/// 场景「add 建户成功」+「首户缺省 root」+「后续户缺省 member」：
/// stdin 喂密（含无尾换行形态），stdout 带确认信息与用户库路径。
#[test]
fn add_creates_users_with_default_roles() {
    let sb = Sandbox::new();
    let out = auth(&sb.db(), &["add", "alice", "--password-stdin"], Some("first-pw-9"));
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("alice"), "{stdout}");
    assert!(stdout.contains("root"), "缺省角色 root 应在确认输出里: {stdout}");
    assert!(stdout.contains("auth.db"), "确认输出应带用户库路径: {stdout}");

    let out = auth(&sb.db(), &["add", "bob", "--password-stdin"], Some("second-pw-9"));
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let store = open_store(&sb.db());
    let alice = store.get_by_username("alice").unwrap().expect("alice 在场");
    assert!(alice.verify_password("first-pw-9"));
    assert_eq!(alice.role, sebas_webui::rbac::Role::Root, "首户缺省 root");
    let bob = store.get_by_username("bob").unwrap().expect("bob 在场");
    assert!(bob.verify_password("second-pw-9"));
    assert_eq!(bob.role, sebas_webui::rbac::Role::Member, "后续户缺省 member");
}

/// 场景「add 同名已存在报错」：`Alice` vs `alice`（大小写不敏感）→ 非零
/// 退出、提示改用 passwd、用户库不变。
#[test]
fn add_duplicate_case_insensitive_points_to_passwd() {
    let sb = Sandbox::new();
    assert!(auth(&sb.db(), &["add", "alice", "--password-stdin"], Some("first-pw-9"))
        .status
        .success());

    let out = auth(
        &sb.db(),
        &["add", "Alice", "--password", "other-pass-9"],
        None,
    );
    assert!(!out.status.success(), "大小写不敏感重名必须失败");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("已存在"), "{stderr}");
    assert!(stderr.contains("auth passwd"), "报错应提示改用 passwd: {stderr}");

    let store = open_store(&sb.db());
    assert_eq!(store.count().unwrap(), 1, "用户库不得新增");
    assert!(
        store
            .get_by_username("alice")
            .unwrap()
            .unwrap()
            .verify_password("first-pw-9"),
        "原用户密码不得被改"
    );
}

/// 场景「非法角色被拒」：词表外角色非零退出且不建户。
#[test]
fn add_rejects_invalid_role() {
    let sb = Sandbox::new();
    let out = auth(
        &sb.db(),
        &["add", "boss", "--role", "superadmin", "--password", "password8"],
        None,
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("root/admin/member/viewer"),
        "报错应带合法词表: {stderr}"
    );
    assert_eq!(open_store(&sb.db()).count().unwrap(), 0, "不得建户");
}

/// 场景「passwd 改密成功」+「passwd 用户不存在报错」：改密后 verify_password
/// 通过、旧密码失效；缺户报错并提示用 add，用户库不变。
#[test]
fn passwd_rotates_and_missing_user_points_to_add() {
    let sb = Sandbox::new();
    assert!(auth(&sb.db(), &["add", "alice", "--password-stdin"], Some("old-pass-9")).status.success());

    let out = auth(&sb.db(), &["passwd", "alice", "--password-stdin"], Some("new-pass-9"));
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("alice"), "{stdout}");

    let store = open_store(&sb.db());
    let alice = store.get_by_username("alice").unwrap().expect("alice 在场");
    assert!(alice.verify_password("new-pass-9"), "新密码应通过校验");
    assert!(!alice.verify_password("old-pass-9"), "旧密码应失效");

    let out = auth(&sb.db(), &["passwd", "ghost", "--password", "password8"], None);
    assert!(!out.status.success(), "不存在的用户必须失败");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("不存在"), "{stderr}");
    assert!(stderr.contains("auth add"), "报错应提示改用 add: {stderr}");
    assert_eq!(store.count().unwrap(), 1, "用户库不得新增");
}

/// 场景「passwd 拒绝 --role」：参数错误非零退出，角色与密码均不变。
#[test]
fn passwd_rejects_role_flag() {
    let sb = Sandbox::new();
    assert!(auth(&sb.db(), &["add", "alice", "--password-stdin"], Some("first-pw-9")).status.success());

    let out = auth(
        &sb.db(),
        &["passwd", "alice", "--role", "admin", "--password", "other-pass-9"],
        None,
    );
    assert!(!out.status.success(), "--role 必须在参数层被拒");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--role") || stderr.contains("unexpected"),
        "clap 参数错误应点名 --role: {stderr}"
    );

    let store = open_store(&sb.db());
    let alice = store.get_by_username("alice").unwrap().unwrap();
    assert!(alice.verify_password("first-pw-9"), "密码不得被改");
    assert_eq!(alice.role, sebas_webui::rbac::Role::Root, "角色不得被改");
}

/// 场景「list 输出账户清单」+「list 零用户」：行含用户名/角色/启用/时间戳，
/// 不含哈希与明文；零用户库输出空。
#[test]
fn list_prints_fields_without_secrets() {
    let sb = Sandbox::new();

    let out = auth(&sb.db(), &["list"], None);
    assert!(out.status.success(), "空库 list 应成功");
    assert!(
        String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        "零用户应如实输出空: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );

    let marker = "list-marker-pw-9";
    assert!(auth(&sb.db(), &["add", "alice", "--password-stdin"], Some(marker)).status.success());
    assert!(
        auth(
            &sb.db(),
            &["add", "vic", "--role", "viewer", "--password", "password8"],
            None
        )
        .status
        .success()
    );

    let out = auth(&sb.db(), &["list"], None);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "两户两行: {stdout}");
    for line in &lines {
        assert!(line.contains("created=") && line.contains("updated="), "{line}");
        assert!(line.contains("enabled"), "{line}");
        assert!(
            !line.contains("salt") && !line.contains("hash"),
            "输出不含盐/哈希字段: {line}"
        );
    }
    assert!(lines[0].contains("alice") && lines[0].contains("root"), "{}", lines[0]);
    assert!(lines[1].contains("vic") && lines[1].contains("viewer"), "{}", lines[1]);
    assert!(
        !stdout.contains(marker),
        "明文密码不得出现在 list 输出: {stdout}"
    );
}

/// 场景「短密码告警不拦截」：5 字符密码建户成功，stderr 有弱密码告警。
#[test]
fn short_password_warns_but_is_allowed() {
    let sb = Sandbox::new();
    let out = auth(&sb.db(), &["add", "admin", "--password", "admin"], None);
    assert!(out.status.success(), "短密码不拦截: {}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning") && stderr.contains("shorter than 8"),
        "stderr 应有弱密码告警: {stderr}"
    );
    let store = open_store(&sb.db());
    assert!(
        store
            .get_by_username("admin")
            .unwrap()
            .unwrap()
            .verify_password("admin")
    );
}

/// 场景「来源互斥报错」：clap 层拒绝（参数错误非零退出）。
#[test]
fn password_sources_are_mutually_exclusive() {
    let sb = Sandbox::new();
    let out = auth(
        &sb.db(),
        &[
            "add",
            "alice",
            "--password",
            "pw",
            "--password-stdin",
        ],
        Some("pw"),
    );
    assert!(!out.status.success(), "两种来源必须互斥");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot be used with") || stderr.contains("互斥"),
        "{stderr}"
    );
}

/// 场景「缺密码报错」：无任何来源 → 非零退出并提示两种来源。
#[test]
fn missing_password_source_names_both_options() {
    let sb = Sandbox::new();
    let out = auth(&sb.db(), &["add", "alice"], None);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("缺少密码"), "{stderr}");
    assert!(stderr.contains("--password-stdin"), "{stderr}");
    assert!(stderr.contains("--password"), "{stderr}");
    assert_eq!(open_store(&sb.db()).count().unwrap(), 0, "不得建户");
}

/// 场景「空密码拒绝」：stdin 空行 → 非零退出报密码不能为空。
#[test]
fn empty_stdin_password_is_rejected() {
    let sb = Sandbox::new();
    let out = auth(&sb.db(), &["add", "alice", "--password-stdin"], Some("\n"));
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("密码不能为空"), "{stderr}");
    assert_eq!(open_store(&sb.db()).count().unwrap(), 0, "不得建户");
}

/// 场景「env 覆盖路径」+「明文不落盘」：库创建在沙箱路径；db 文件字节里
/// 检索不到明文密码，只有盐与哈希。
#[test]
fn env_overrides_db_path_and_plaintext_never_hits_disk() {
    let sb = Sandbox::new();
    let marker = "on-disk-marker-pw-42";
    let out = auth(&sb.db(), &["add", "alice", "--password-stdin"], Some(marker));
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let db = sb.db();
    assert!(db.exists(), "用户库应创建在 SEBAS_WEBUI_AUTH_DB 沙箱路径");
    let raw_bytes = std::fs::read(&db).unwrap();
    let raw = String::from_utf8_lossy(&raw_bytes);
    assert!(
        !raw.contains(marker),
        "auth.db 里检索到明文密码——落盘必须只有盐与哈希"
    );
    assert!(raw.contains("salt_hex"), "盐列应存在");
    assert!(raw.contains("hash_hex"), "哈希列应存在");
}

/// cli-service spec 场景「retired webui-passwd rejected」：旧子命令未知、
/// 非零退出；账户管理改用 `sebas auth`。
#[test]
fn retired_webui_passwd_is_unknown_subcommand() {
    let sb = Sandbox::new();
    let out = run_sebas(
        &sb.db(),
        &["webui-passwd", "--user", "admin", "--password", "admin"],
        None,
    );
    assert!(!out.status.success(), "退役命令必须失败");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("webui-passwd"), "{stderr}");
    assert_eq!(open_store_or_empty(&sb.db()), 0, "退役命令不得写库");
}

/// 退役命令没建库（文件可能根本不存在）时的计数兜底。
fn open_store_or_empty(db: &Path) -> i64 {
    UserStore::open_with_iterations(db, 1000)
        .and_then(|s| s.count())
        .unwrap_or(0)
}
