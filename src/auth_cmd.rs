//! `sebas auth` — WebUI 账户体系的 CLI 管理面（add-auth-subcommand；
//! openspec/changes/add-auth-subcommand/specs/auth-cli/spec.md）。
//!
//! 三个动词、语义按动词拆分（spec「auth 组命令形态」，不再有
//! create-or-update 一体形态）：
//!
//! ```text
//! $ printf '%s' 'pw' | sebas auth add alice          # 零用户库 → root
//! WebUI 登录用户已创建：用户 alice（角色 root，C:\Users\me\.sebas\auth.db）
//! $ printf '%s' 'new-pw' | sebas auth passwd alice   # 只改密（新盐新哈希）
//! WebUI 密码已更新：用户 alice（C:\Users\me\.sebas\auth.db）
//! $ sebas auth list                                  # 只读列表，无哈希
//! alice  root  enabled  created=1727222400  updated=1727222400
//! ```
//!
//! - `add` 只建户：同名（大小写不敏感，存储层 `COLLATE NOCASE`）已存在 →
//!   报错并提示用 `auth passwd`；缺省角色零用户 → root、否则 member，
//!   `--role` 显式覆盖。
//! - `passwd` 只改密：用户不存在 → 报错并提示用 `auth add`；不携带
//!   `--role`（角色调整归 WebUI root 管理面）。
//! - 密码来源 `--password-stdin`（读一行，去尾部 CR/LF）或 `--password`，
//!   互斥；皆无或空密码 → 报错；<8 字符仅 stderr 告警不拦截（CLI 面向
//!   测试环境与操作者自主权衡，与首启设置页的 ≥8 硬门槛不同）。明文
//!   绝不落盘。
//! - 用户库路径 env-only：`SEBAS_WEBUI_AUTH_DB`（缺省 `~/.sebas/auth.db`），
//!   与运行中 webui 同源；不读 config.toml。用户库即活数据，变更即时生效。

use crate::error::{Result, SebasError};
use sebas_webui::auth::default_auth_db;
use sebas_webui::rbac::Role;
use sebas_webui::user_store::{StoreError, UserInfo, UserStore};

/// `sebas auth` 的参数（无 config：路径 env-only，与运行时同源——
/// auth-cli spec「用户库路径与生效时机」）。
pub struct AuthArgs {
    pub cmd: AuthCmd,
}

/// `sebas auth` 子命令（与 cli::AuthCmd 一一对应，main.rs 转译）。
pub enum AuthCmd {
    /// 建户：缺省角色（零用户 → root、否则 member），`--role` 显式覆盖。
    Add {
        username: String,
        role: Option<String>,
        password: Option<String>,
        password_stdin: bool,
    },
    /// 改密（新盐新哈希）；用户不存在报错。无 `--role`。
    Passwd {
        username: String,
        password: Option<String>,
        password_stdin: bool,
    },
    /// 只读列表：用户名 / 角色 / 启用 / 时间戳（不含哈希）。
    List,
}

/// CLI 入口：一次性管理命令（非服务），失败按普通错误退出 1。
pub fn run(args: AuthArgs) -> Result<()> {
    match args.cmd {
        AuthCmd::Add {
            username,
            role,
            password,
            password_stdin,
        } => add(&username, role.as_deref(), password, password_stdin),
        AuthCmd::Passwd {
            username,
            password,
            password_stdin,
        } => passwd(&username, password, password_stdin),
        AuthCmd::List => list(),
    }
}

/// `sebas auth add <USER>`：只建户。缺省角色：库零用户 → root，否则
/// member；`--role`（root/admin/member/viewer 四档）显式给出时覆盖缺省。
/// 同名（大小写不敏感）已存在 → 报错并提示改用 `sebas auth passwd`。
pub fn add(
    username: &str,
    role: Option<&str>,
    password: Option<String>,
    password_stdin: bool,
) -> Result<()> {
    add_at(&default_auth_db(), username, role, password, password_stdin)
}

/// [`add`] 的路径注入形态（单测钉 tempdir 沙箱用——不碰进程级 env，
/// 避免与 lib 内其他 env 型测试互踩）。
pub(crate) fn add_at(
    db: &std::path::Path,
    username: &str,
    role: Option<&str>,
    password: Option<String>,
    password_stdin: bool,
) -> Result<()> {
    let username = validate_username(username)?;
    let explicit_role = parse_role(role)?;
    let password = resolve_password(password, password_stdin)?;
    let store = open_store_at(db)?;

    let role = explicit_role.unwrap_or(if count_users(&store)? == 0 {
        Role::Root
    } else {
        Role::Member
    });
    store
        .create(&username, &password, role)
        .map_err(|e| match e {
            StoreError::UsernameTaken => SebasError::Config(format!(
                "用户名 {username} 已存在，请改用 `sebas auth passwd {username}` 改密"
            )),
            other => SebasError::Config(format!("建户失败: {other}")),
        })?;
    println!(
        "WebUI 登录用户已创建：用户 {username}（角色 {role}，{}）\n现在 webui 的全部 API/WebSocket 都需要登录；\
         若需公网部署，把 [service.webui] host 指到 0.0.0.0 即可。",
        db.display()
    );
    Ok(())
}

/// `sebas auth passwd <USER>`：只改密（写入新盐新哈希）。用户不存在 →
/// 报错并提示改用 `sebas auth add`。无 `--role`：clap 层就不接受（unexpected
/// argument），角色调整归 WebUI root 管理面。用户库即活数据：运行中的
/// webui 进程按请求实时读库，改密即时生效。
pub fn passwd(username: &str, password: Option<String>, password_stdin: bool) -> Result<()> {
    passwd_at(&default_auth_db(), username, password, password_stdin)
}

/// [`passwd`] 的路径注入形态（单测钉 tempdir 沙箱用）。
pub(crate) fn passwd_at(
    db: &std::path::Path,
    username: &str,
    password: Option<String>,
    password_stdin: bool,
) -> Result<()> {
    let username = validate_username(username)?;
    let password = resolve_password(password, password_stdin)?;
    let store = open_store_at(db)?;

    let user = store
        .get_by_username(&username)
        .map_err(|e| SebasError::Config(format!("查询用户库失败: {e}")))?
        .ok_or_else(|| {
            SebasError::Config(format!(
                "用户 {username} 不存在，请先用 `sebas auth add {username}` 建户"
            ))
        })?;
    store
        .set_password(user.id, &password)
        .map_err(|e| SebasError::Config(format!("改密失败: {e}")))?;
    // 打存储里的规范用户名（NOCASE 命中时可能与输入大小写不同）。
    println!("WebUI 密码已更新：用户 {}（{}）", user.username, db.display());
    Ok(())
}

/// `sebas auth list`：只读列表（用户名 / 角色 / 启用 / 时间戳，无哈希）。
/// 零用户如实输出空（spec「list 零用户」：不编造条目）。
pub fn list() -> Result<()> {
    list_at(&default_auth_db())
}

/// [`list`] 的路径注入形态（单测钉 tempdir 沙箱用）。
pub(crate) fn list_at(db: &std::path::Path) -> Result<()> {
    let store = open_store_at(db)?;
    let users = store
        .list()
        .map_err(|e| SebasError::Config(format!("查询用户库失败: {e}")))?;
    for line in format_user_lines(&users) {
        println!("{line}");
    }
    Ok(())
}

/// 打开用户库（不存在则创建并初始化为零用户状态）。
fn open_store_at(db: &std::path::Path) -> Result<UserStore> {
    UserStore::open(db)
        .map_err(|e| SebasError::Config(format!("打开 WebUI 用户库 {} 失败: {e}", db.display())))
}

/// 库内用户数（决定 `add` 的缺省角色）。
fn count_users(store: &UserStore) -> Result<i64> {
    store
        .count()
        .map_err(|e| SebasError::Config(format!("查询用户库失败: {e}")))
}

/// 用户名位置参数校验（存储层还会再 trim/拒空，这里先给可读报错）。
fn validate_username(raw: &str) -> Result<String> {
    let username = raw.trim();
    if username.is_empty() {
        return Err(SebasError::Config(
            "用户名不能为空：`sebas auth add <USER>` / `sebas auth passwd <USER>`".into(),
        ));
    }
    Ok(username.to_string())
}

/// `--role` 解析：root/admin/member/viewer 四档、大小写敏感，词表外报错
/// （spec「非法角色被拒」）。
fn parse_role(role: Option<&str>) -> Result<Option<Role>> {
    role.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|word| {
            word.parse::<Role>()
                .map_err(|e| SebasError::Config(format!("--role 非法: {e}")))
        })
        .transpose()
}

/// 从流里读密码：取首行、去行尾 CR/LF（Windows 管道 CRLF 不进密码）。
/// `printf '%s' 'pw'`（无换行）与 `printf '%s\n' 'pw'` 等价；多余行不进
/// 密码（spec「stdin 读密」：首行即密码）。
fn read_password_from(r: &mut dyn std::io::Read) -> Result<String> {
    let mut raw = String::new();
    r.read_to_string(&mut raw)
        .map_err(|e| SebasError::Config(format!("read password from stdin: {e}")))?;
    let line = raw.split('\n').next().unwrap_or("");
    Ok(line.trim_end_matches('\r').to_string())
}

/// 密码来源解析（add/passwd 共享，spec「密码来源与校验」）：`--password`
/// 与 `--password-stdin` 互斥（clap 层已拒，直调路径在此兜底）；皆无 →
/// 报错提示两种来源；空密码拒绝；<8 字符仅 stderr 告警不拦截（测试环境
/// 统一 admin/admin 是合法形态；公网部署由操作者自行权衡强度）。
fn resolve_password(password: Option<String>, password_stdin: bool) -> Result<String> {
    if password.is_some() && password_stdin {
        return Err(SebasError::Config(
            "--password 与 --password-stdin 互斥：只能给一种密码来源".into(),
        ));
    }
    let password = match password {
        Some(p) => p,
        None if password_stdin => {
            let mut stdin = std::io::stdin().lock();
            read_password_from(&mut stdin)?
        }
        None => {
            return Err(SebasError::Config(
                "缺少密码：用 --password-stdin（推荐，避免进 shell history）或 --password".into(),
            ))
        }
    };
    if password.is_empty() {
        return Err(SebasError::Config("密码不能为空".into()));
    }
    if password.chars().count() < 8 {
        // stderr 直出：一次性命令不初始化 tracing，warn! 会静默丢弃；
        // spec 场景「短密码告警不拦截」要求告警可见。不做硬性拦截。
        eprintln!(
            "warning: password is shorter than 8 chars, weak; use a strong password for public deploys"
        );
    }
    Ok(password)
}

/// list 的输出行（纯格式化，导出供测试断言排版）：
/// `用户名  角色  enabled|disabled  created=<unix>  updated=<unix>`。
/// 只有这五类字段——盐/哈希（UserRecord 独有）从不进输出（spec
/// 「不含密码哈希」）。
pub fn format_user_lines(users: &[UserInfo]) -> Vec<String> {
    users
        .iter()
        .map(|u| {
            format!(
                "{}  {}  {}  created={}  updated={}",
                u.username,
                u.role,
                if u.enabled { "enabled" } else { "disabled" },
                u.created_at_unix,
                u.updated_at_unix
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! auth-cli spec 场景的单元层覆盖：来源互斥 / 缺密码 / 空密码 / 短密码
    //! 告警 / stdin 去尾 CR-LF / 缺省角色与显式覆盖 / 重名大小写不敏感 /
    //! 改密生效 / list 无哈希无明文。
    //!
    //! 沙箱姿态：一律走 `*_at` 路径注入形态 + tempdir——**不设**进程级
    //! `SEBAS_WEBUI_AUTH_DB`。env 是进程全局的，lib 测试二进制里还有别的
    //! env 型用例（webui_cmd::auth_gate_tests），两把锁抢一个 env 必互踩；
    //! 显式路径让本模块用例无 env 依赖、可全并行。env 解析路径（含缺省
    //! `~/.sebas/auth.db` 回落）由 tests/auth_cli_test.rs 以逐子进程显式
    //! env 覆盖（真二进制，spec「env 覆盖路径」场景）。

    use super::*;
    use sebas_webui::user_store::UserStore;
    use std::io::Cursor;
    use std::path::Path;

    /// 沙箱验证用只读开库（迭代数不影响已存用户的校验路径）。
    fn open_store_at(db: &Path) -> UserStore {
        UserStore::open_with_iterations(db, 1000).expect("open sandbox store")
    }

    // ── 密码来源与校验（spec「密码来源与校验」场景） ─────────────────────

    /// 场景「来源互斥报错」：同时给出两种来源 → 参数错误。
    #[test]
    fn password_sources_are_mutually_exclusive() {
        let err = resolve_password(Some("pw".into()), true).expect_err("互斥必须报错");
        assert!(err.to_string().contains("互斥"), "{err}");
    }

    /// 场景「缺密码报错」：皆无 → 报错并提示两种来源。
    #[test]
    fn missing_password_source_names_both_options() {
        let err = resolve_password(None, false).expect_err("缺来源必须报错");
        let msg = err.to_string();
        assert!(msg.contains("--password-stdin"), "{msg}");
        assert!(msg.contains("--password"), "{msg}");
    }

    /// 场景「空密码拒绝」：`--password ""` → 报密码不能为空。
    #[test]
    fn empty_password_is_rejected() {
        let err = resolve_password(Some(String::new()), false).expect_err("空密码必须报错");
        assert!(err.to_string().contains("密码不能为空"), "{err}");
    }

    /// 场景「短密码告警不拦截」：5 字符密码照常通过（告警语义无法在
    /// 单测断言 stderr，由集成测试覆盖）。
    #[test]
    fn short_password_is_allowed() {
        resolve_password(Some("admin".into()), false).expect("短密码不拦截");
    }

    /// 场景「stdin 读密」：首行去尾部 CR/LF；无换行等价；多余行不进密码。
    #[test]
    fn stdin_password_reads_first_line_without_crlf() {
        let cases: [(&str, &str); 5] = [
            ("pw", "pw"),       // 无换行（printf '%s'）
            ("pw\n", "pw"),     // 单换行
            ("pw\r\n", "pw"),   // Windows CRLF
            ("pw\r\r\n", "pw"), // 连续尾部 CR
            ("pw\nrest", "pw"), // 首行即密码，多余行不进
        ];
        for (input, want) in cases {
            let got = read_password_from(&mut Cursor::new(input)).unwrap();
            assert_eq!(got, want, "input={input:?}");
        }
        // 空流 → 空串（交给 resolve_password 的空密码拒绝兜底）。
        let got = read_password_from(&mut Cursor::new("")).unwrap();
        assert_eq!(got, "");
    }

    // ── add：缺省角色与显式角色（spec「缺省角色与显式角色」场景） ─────────

    /// 场景「首户缺省 root」+「后续户缺省 member」+「add 建户成功」。
    #[test]
    fn add_first_user_defaults_root_then_member() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("auth.db");

        add_at(&db, "admin", None, Some("admin".into()), false).expect("首户应创建成功");
        add_at(&db, "bob", None, Some("password8".into()), false).expect("第二户应创建成功");

        let store = open_store_at(&db);
        let admin = store.get_by_username("admin").unwrap().expect("admin 在场");
        assert_eq!(admin.role, Role::Root, "零用户库首户缺省 root");
        assert!(admin.verify_password("admin"), "短密码仅告警不拦截");
        let bob = store.get_by_username("bob").unwrap().expect("bob 在场");
        assert_eq!(bob.role, Role::Member, "非零用户库缺省 member");
        assert!(bob.verify_password("password8"));
    }

    /// 场景「--role 显式覆盖」+「非法角色被拒」（词表外不建户）。
    #[test]
    fn add_explicit_role_overrides_and_invalid_role_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("auth.db");

        add_at(&db, "vic", Some("viewer"), Some("password8".into()), false).expect("viewer 建户");
        add_at(&db, "ada", Some("admin"), Some("password8".into()), false).expect("admin 建户");

        let store = open_store_at(&db);
        assert_eq!(
            store.get_by_username("vic").unwrap().unwrap().role,
            Role::Viewer
        );
        assert_eq!(
            store.get_by_username("ada").unwrap().unwrap().role,
            Role::Admin
        );

        let err = add_at(&db, "boss", Some("superadmin"), Some("password8".into()), false)
            .expect_err("非法角色必须报错");
        assert!(
            err.to_string().contains("root/admin/member/viewer"),
            "报错应带合法词表: {err}"
        );
        assert!(store.get_by_username("boss").unwrap().is_none(), "不得建户");
    }

    /// 场景「add 同名已存在报错」：`Alice` vs `alice`（大小写不敏感）→
    /// 非零错误、提示改用 passwd、用户库不变。
    #[test]
    fn add_duplicate_case_insensitive_points_to_passwd() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("auth.db");

        add_at(&db, "alice", None, Some("password8".into()), false).expect("建 alice");
        let err = add_at(&db, "Alice", None, Some("other-pass-9".into()), false)
            .expect_err("大小写不敏感重名必须报错");
        let msg = err.to_string();
        assert!(msg.contains("已存在"), "{msg}");
        assert!(msg.contains("auth passwd"), "报错应提示改用 passwd: {msg}");

        let store = open_store_at(&db);
        assert_eq!(store.count().unwrap(), 1, "用户库不得新增");
        assert!(
            store
                .get_by_username("alice")
                .unwrap()
                .unwrap()
                .verify_password("password8"),
            "原用户密码不得被改"
        );
    }

    // ── passwd：改密与缺户（spec「passwd 改密成功 / 用户不存在报错」） ────

    /// 场景「passwd 改密成功」：新盐新哈希，旧密码失效。
    #[test]
    fn passwd_rotates_password() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("auth.db");

        add_at(&db, "alice", None, Some("old-pass-9".into()), false).expect("建户");
        passwd_at(&db, "alice", Some("new-pass-9".into()), false).expect("改密应成功");

        let store = open_store_at(&db);
        let user = store.get_by_username("alice").unwrap().expect("alice 在场");
        assert!(user.verify_password("new-pass-9"), "新密码应通过校验");
        assert!(!user.verify_password("old-pass-9"), "旧密码应失效");
    }

    /// 场景「passwd 用户不存在报错」：提示改用 add、用户库不变。
    #[test]
    fn passwd_missing_user_points_to_add() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("auth.db");

        add_at(&db, "alice", None, Some("password8".into()), false).expect("建户");
        let err = passwd_at(&db, "ghost", Some("password8".into()), false)
            .expect_err("不存在的用户必须报错");
        let msg = err.to_string();
        assert!(msg.contains("不存在"), "{msg}");
        assert!(msg.contains("auth add"), "报错应提示改用 add: {msg}");

        let store = open_store_at(&db);
        assert_eq!(store.count().unwrap(), 1, "用户库不得新增");
    }

    // ── list：字段与脱敏（spec「list 输出账户清单 / 零用户」） ────────────

    /// 场景「list 输出账户清单」：用户名/角色/启用/时间戳齐备，不含哈希
    /// 与明文密码；场景「list 零用户」：空库输出空。
    #[test]
    fn user_lines_carry_fields_but_never_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("auth.db");
        let store = open_store_at(&db);

        // 零用户：空输出，不编造条目。
        assert_eq!(store.list().unwrap().len(), 0);
        assert!(format_user_lines(&store.list().unwrap()).is_empty());

        let marker_password = "un-guessable-marker-pw";
        store.create("alice", marker_password, Role::Root).unwrap();
        store.create("vic", "password8", Role::Viewer).unwrap();
        let lines = format_user_lines(&store.list().unwrap());
        assert_eq!(lines.len(), 2);

        let alice = &lines[0];
        assert!(alice.contains("alice"), "{alice}");
        assert!(alice.contains("root"), "{alice}");
        assert!(alice.contains("enabled"), "{alice}");
        assert!(alice.contains("created="), "{alice}");
        assert!(alice.contains("updated="), "{alice}");
        // 无哈希：盐/哈希列的十六进制形态不进输出；明文密码也不进。
        for line in &lines {
            assert!(
                !line.contains(marker_password),
                "明文密码不得出现在输出: {line}"
            );
            assert!(
                !line.contains("salt") && !line.contains("hash"),
                "输出不含盐/哈希字段: {line}"
            );
        }
    }

    /// 场景「明文不落盘」：建库后扫描 db 文件字节，检索不到明文密码。
    #[test]
    fn plaintext_password_never_hits_disk() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("auth.db");

        let marker = "on-disk-marker-pw-42";
        add_at(&db, "alice", None, Some(marker.into()), false).expect("建户");

        let raw_bytes = std::fs::read(&db).expect("auth.db 应创建在沙箱");
        let raw = String::from_utf8_lossy(&raw_bytes);
        assert!(
            !raw.contains(marker),
            "auth.db 里检索到明文密码——落盘必须只有盐与哈希"
        );
        // 盐与哈希在场（十六进制列非空）。
        assert!(raw.contains("salt_hex"), "盐列应存在");
        assert!(raw.contains("hash_hex"), "哈希列应存在");
    }
}
