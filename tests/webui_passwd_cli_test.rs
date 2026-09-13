//! `sebas webui-passwd` 集成测试（add-webui-multiuser-rbac 4.1，design D7）：
//! 新建（首个用户默认 root、其后默认 member）、改密、`--role` 显式覆盖与
//! 已存在用户的角色更新、最后 root 保护。全部经 `SEBAS_WEBUI_AUTH_DB` 落
//! tempdir 沙箱，不触真实 `~/.sebas`；旧 JSON 凭据文件不再被读写。

use sebas::webui_cmd::{WebUiPasswdArgs, run_passwd};
use sebas_webui::rbac::Role;
use sebas_webui::user_store::UserStore;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// env 是进程全局的：所有用例共用一把锁串行（与 webui_cmd::auth_gate_tests
/// 同一姿态）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct AuthDbGuard(PathBuf);
fn set_auth_db(dir: &Path) -> AuthDbGuard {
    let db = dir.join("auth.db");
    // SAFETY: ENV_LOCK 由调用方持有，无并发 env 访问。
    unsafe {
        std::env::set_var("SEBAS_WEBUI_AUTH_DB", &db);
    }
    AuthDbGuard(db)
}
impl Drop for AuthDbGuard {
    fn drop(&mut self) {
        // SAFETY: 同上，ENV_LOCK 仍被持有。
        unsafe {
            std::env::remove_var("SEBAS_WEBUI_AUTH_DB");
        }
    }
}

fn passwd_args(user: &str, password: &str, role: Option<&str>) -> WebUiPasswdArgs {
    WebUiPasswdArgs {
        user: Some(user.into()),
        password: Some(password.into()),
        password_stdin: false,
        role: role.map(str::to_string),
    }
}

fn open_store(db: &Path) -> UserStore {
    UserStore::open_with_iterations(db, 1000).expect("open sandbox store")
}

/// 首个用户默认 root、其后默认 member（design D7），且不写任何 JSON 凭据
/// 文件——目录里只有 auth.db（及 SQLite 的 -wal/-shm）。
#[test]
fn first_user_defaults_root_then_member() {
    let _env = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let guard = set_auth_db(dir.path());

    run_passwd(passwd_args("admin", "admin", None)).expect("首个用户应创建成功");
    run_passwd(passwd_args("bob", "password8", None)).expect("第二个用户应创建成功");

    let store = open_store(&guard.0);
    let admin = store
        .get_by_username("admin")
        .unwrap()
        .expect("admin 应在场");
    assert_eq!(admin.role, Role::Root, "首个用户默认 root");
    assert!(admin.verify_password("admin"), "短密码仅告警不拦截");
    let bob = store.get_by_username("bob").unwrap().expect("bob 应在场");
    assert_eq!(bob.role, Role::Member, "其后默认 member");
    assert!(bob.verify_password("password8"));

    // 不再写 JSON：沙箱目录里没有任何 webui-auth.json。
    let stray: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name.contains("json"))
        .collect();
    assert!(stray.is_empty(), "不得写 JSON 凭据文件: {stray:?}");
}

/// `--role` 显式覆盖建户默认值；大小写敏感的词表外角色报错。
#[test]
fn explicit_role_overrides_creation_default() {
    let _env = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let guard = set_auth_db(dir.path());

    run_passwd(passwd_args("vic", "password8", Some("viewer"))).expect("显式 viewer 建户");
    run_passwd(passwd_args("ada", "password8", Some("admin"))).expect("显式 admin 建户");

    let store = open_store(&guard.0);
    assert_eq!(
        store.get_by_username("vic").unwrap().unwrap().role,
        Role::Viewer
    );
    assert_eq!(
        store.get_by_username("ada").unwrap().unwrap().role,
        Role::Admin
    );

    // 词表外角色：命令失败且不建户。
    let err = run_passwd(passwd_args("boss", "password8", Some("superadmin")))
        .expect_err("非法角色必须报错");
    assert!(err.to_string().contains("root/admin/member/viewer"), "{err}");
    assert!(store.get_by_username("boss").unwrap().is_none());
}

/// 对已存在用户重跑 = 改密（新盐新哈希，角色保持）；带 `--role` 则同时
/// 改角色。最后一个启用的 root 降级被拒（存储层 LastRoot 保护透传）。
#[test]
fn rerun_resets_password_and_role_flag_updates_role() {
    let _env = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let guard = set_auth_db(dir.path());

    run_passwd(passwd_args("root", "password8", None)).expect("建 root");
    run_passwd(passwd_args("member1", "password8", None)).expect("建 member");

    let store = open_store(&guard.0);
    let root_id = store.get_by_username("root").unwrap().unwrap().id;
    let member1_id = store.get_by_username("member1").unwrap().unwrap().id;

    // 改密：角色不变、旧密码失效。
    run_passwd(passwd_args("root", "rotated-pass-9", None)).expect("改密应成功");
    let root = store.get(root_id).unwrap();
    assert!(root.verify_password("rotated-pass-9"));
    assert!(!root.verify_password("password8"));
    assert_eq!(root.role, Role::Root);

    // --role 对已存在用户：member1 升 admin。
    run_passwd(passwd_args("member1", "password8", Some("admin"))).expect("改角色应成功");
    assert_eq!(store.get(member1_id).unwrap().role, Role::Admin);

    // 最后启用的 root 降级：存储层 LastRoot 拒绝，命令失败、root 原状。
    let err = run_passwd(passwd_args("root", "rotated-pass-9", Some("member")))
        .expect_err("降级最后一个 root 必须失败");
    assert!(
        err.to_string().contains("最后一个启用的 root"),
        "错误应透传 LastRoot 保护: {err}"
    );
    assert_eq!(store.get(root_id).unwrap().role, Role::Root);
}

/// 旧 JSON 凭据文件即使躺在旁边也不被读取或写入（建户照常走 auth.db，
/// JSON 内容原样保留）。
#[test]
fn legacy_json_file_is_never_read_or_written() {
    let _env = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let guard = set_auth_db(dir.path());

    let legacy = dir.path().join("webui-auth.json");
    std::fs::write(
        &legacy,
        r#"{"username":"legacy-admin","password":"legacy-pw"}"#,
    )
    .unwrap();

    run_passwd(passwd_args("fresh", "password8", None)).expect("建户应成功且不受旧文件影响");

    let store = open_store(&guard.0);
    assert!(
        store.get_by_username("legacy-admin").unwrap().is_none(),
        "旧 JSON 用户不得被迁移进用户库"
    );
    let raw = std::fs::read_to_string(&legacy).unwrap();
    assert!(raw.contains("legacy-admin"), "旧 JSON 不得被改写: {raw}");
    assert_eq!(store.count().unwrap(), 1);
}
