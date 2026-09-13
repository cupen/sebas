//! WebUI 登录鉴权：多用户「用户名 / 密码」+ RBAC 身份解析
//! （add-webui-multiuser-rbac 2.1/2.2，design D2/D5）。
//!
//! # 账户模型
//!
//! 用户存独立 SQLite 库 auth.db（[`crate::user_store`]，默认
//! `~/.sebas/auth.db`，`SEBAS_WEBUI_AUTH_DB` 覆盖）。DB 即活数据：旧单账户
//! 时代的「凭据文件 + mtime 热重载」机制整体移除，用户管理/改密/禁用经
//! [`UserStore`] 写库即时生效。`webui-auth.json` / `SEBAS_WEBUI_AUTH_FILE` /
//! `SEBAS_WEBUI_TOKEN` / `{"secret"}` 单字段登录全部不再支持（旧系统不做
//! 兼容，未正式发布）。
//!
//! # 登录
//!
//! 只接受用户名 + 密码（design D5）：按用户名查库验 PBKDF2；用户名不存在
//! 也跑一次等代价哑哈希，抹平时序差防用户名枚举。禁用用户的登录拒绝与
//! 凭据错误同文案（HTTP 401，由 handler 统一映射）。
//!
//! # 生命周期
//!
//! - 引导：零用户时 `POST /api/auth/setup`（[`AuthHandle::setup_root`]，
//!   事务内零用户校验）或 `SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD`
//!   env（`webui_cmd::bootstrap_auth`）。不再自动生成随机密码。
//! - 会话：登录/建 root 成功即建会话并绑定用户 id；每请求经
//!   [`AuthHandle::identity_for_session`] 实时解析 enabled + role
//!   （角色调整即时生效、禁用/删号即刻失效——spec「会话绑定用户」）。
//! - 损坏姿态：auth.db 打不开时不静默降级为免鉴权——句柄保持 enabled、
//!   一切登录/身份解析一律拒绝并报错日志。
//!
//! 会话与限速复用 [`crate::admin_auth::SessionStore`]（24h 不活动 TTL、
//! per-IP 登录限速）。

use crate::admin_auth::SessionStore;
use crate::rbac::Role;
use crate::user_store::{StoreError, UserStore};
use std::path::{Path, PathBuf};

// 哈希原语与默认库路径实际住在 user_store（存储自管密码材料）；这里
// 再导出，历史调用点（admin_auth::generate_token 等）继续走 crate::auth。
pub use crate::user_store::{default_auth_db, pbkdf2_hmac_sha256, random_bytes, PBKDF2_ITERATIONS};

/// WebUI 会话 cookie 名（HttpOnly + SameSite=Lax）。
pub const SESSION_COOKIE_NAME: &str = "sebas_webui_session";

/// 首启 setup 的最小密码长度（spec：不满足即 400，不做静默降级）。
pub const MIN_PASSWORD_LEN: usize = 8;

/// 登录失败原因（HTTP 语义由 handler 决定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginError {
    /// 鉴权关闭（`[watchdog.webui] auth = false`），登录面不存在。
    Disabled,
    /// 用户名或密码错误。
    Invalid,
    /// 账户被禁用（spec：与凭据错误同文案 401，handler 侧 `Err(_)` 同路）。
    AccountDisabled,
    /// 该来源 IP 触发登录限速。
    RateLimited,
}

/// 首启建 root 的失败原因（`POST /api/auth/setup` → 409/400 映射的依据）。
/// 不 derives PartialEq/Clone（`Store` 变体嵌套的 rusqlite 错误两者皆缺），
/// 测试用 `matches!` 断言。
#[derive(Debug)]
pub enum SetupError {
    /// 鉴权关闭（disabled handle），无设置页形态。
    Disabled,
    /// 用户库已有用户（→ 409）。
    AlreadySetup,
    /// 密码短于 [`MIN_PASSWORD_LEN`]（→ 400）。
    WeakPassword,
    /// 用户库不可用（打开失败 / 存储层错误）。
    Unavailable,
    /// 存储层语义错误（用户名撞车/为空等，透传给 handler 映射）。
    Store(StoreError),
}

/// 已认证会话的实时身份（每请求解析，角色不快照进会话——design D3：
/// auth_guard 把它塞进 request extensions 供 handler 读取）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub user_id: i64,
    pub username: String,
    pub role: Role,
}

/// 服务端鉴权共享态：用户库 + 会话存储。
///
/// `users` 为 `None` 的两种形态：`enabled = false`（鉴权开关关闭，全路由
/// 免登录）与「打开失败」的降级态（`enabled = true` 但一切登录/身份解析
/// 拒绝，见模块文档「损坏姿态」）。
pub struct AuthHandle {
    enabled: bool,
    path: PathBuf,
    users: Option<UserStore>,
    pub session_store: SessionStore,
}

impl AuthHandle {
    /// 鉴权关闭态（`auth = false` 与测试接线用）：不触盘，全路由免登录。
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            path: PathBuf::new(),
            users: None,
            session_store: SessionStore::new(),
        }
    }

    /// 打开（必要时创建）auth.db。不 panic：打不开 = 降级态（enabled 但
    /// 一律拒绝登录），错误进日志（design Risks：不静默降级为免鉴权）。
    pub fn open(path: PathBuf) -> Self {
        Self::open_with_iterations(path, PBKDF2_ITERATIONS)
    }

    /// 同 [`Self::open`] 但指定新用户默认 PBKDF2 迭代次数（测试提速用）。
    pub fn open_with_iterations(path: PathBuf, iterations: u32) -> Self {
        match UserStore::open_with_iterations(&path, iterations) {
            Ok(users) => Self {
                enabled: true,
                path,
                users: Some(users),
                session_store: SessionStore::new(),
            },
            Err(e) => {
                tracing::error!(
                    path = %path.display(),
                    error = %e,
                    "failed to open webui auth.db, logins and identities will be rejected"
                );
                Self {
                    enabled: true,
                    path,
                    users: None,
                    session_store: SessionStore::new(),
                }
            }
        }
    }

    /// auth.db 路径（disabled 态为空）。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 鉴权是否开启（`[watchdog.webui] auth` 接线的实况）。与旧单账户时代
    /// 不同：这只反映开关/句柄形态，与「库里有没有用户」无关——零用户是
    /// 首启设置页形态（[`Self::needs_setup`]），不是免鉴权。
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// 用户库句柄（用户管理端点与引导路径用）；disabled / 降级态为 None。
    pub fn user_store(&self) -> Option<&UserStore> {
        self.users.as_ref()
    }

    /// 是否处于「鉴权开启但零用户」的首启设置页形态。降级态恒 false
    /// （对着坏库渲染设置页只会误导）。
    pub fn needs_setup(&self) -> bool {
        match (&self.users, self.enabled) {
            (Some(users), true) => users.count().map(|c| c == 0).unwrap_or(false),
            _ => false,
        }
    }

    /// 校验用户名 + 密码并创建绑定该用户的会话，返回会话 cookie 值。
    /// 限速按来源 IP（与 admin 登录共用 SessionStore 的限速逻辑）。
    ///
    /// 时序安全：用户名不存在也跑一次等代价哑 PBKDF2（design D5），
    /// 避免「不存在直接返回 vs 密码错跑 120k 次哈希」的快慢差被用来
    /// 枚举用户名。
    pub async fn login(
        &self,
        client_ip: &str,
        username: &str,
        password: &str,
    ) -> Result<String, LoginError> {
        if !self.enabled {
            return Err(LoginError::Disabled);
        }
        let Some(users) = &self.users else {
            // 降级态（库打不开）：一律拒绝（不消耗限速计数——没有可验证
            // 的凭据面，暴力破解无从谈起）。
            return Err(LoginError::Invalid);
        };
        if !self.session_store.check_rate_limit(client_ip).await {
            return Err(LoginError::RateLimited);
        }
        let verified = match users.get_by_username(username) {
            Ok(Some(user)) => {
                if user.verify_password(password) {
                    Some(user)
                } else {
                    None
                }
            }
            Ok(None) => {
                users.dummy_verify_delay(password);
                None
            }
            Err(_) => {
                // 库层错误：同样哑哈希后拒绝（坏库不静默放行）。
                users.dummy_verify_delay(password);
                None
            }
        };
        let Some(user) = verified else {
            return Err(LoginError::Invalid);
        };
        if !user.enabled {
            return Err(LoginError::AccountDisabled);
        }
        self.session_store.reset_rate_limit(client_ip).await;
        let (session_id, _csrf) = self.session_store.create(user.id).await;
        Ok(session_id)
    }

    /// 首启建 root（`POST /api/auth/setup` 的服务端核，design D4）：
    /// 事务内零用户校验 + 建 root + 建会话（成功后自动进入工作台）。
    /// 密码短于 [`MIN_PASSWORD_LEN`] → [`SetupError::WeakPassword`]（400；
    /// env/CLI 引导不走这里、不受此拦截）。非零用户 →
    /// [`SetupError::AlreadySetup`]（409）。
    pub async fn setup_root(&self, username: &str, password: &str) -> Result<String, SetupError> {
        if !self.enabled {
            return Err(SetupError::Disabled);
        }
        let Some(users) = &self.users else {
            return Err(SetupError::Unavailable);
        };
        if password.chars().count() < MIN_PASSWORD_LEN {
            return Err(SetupError::WeakPassword);
        }
        let user_id = users
            .setup_root(username, password)
            .map_err(|e| match e {
                StoreError::AlreadyInitialized => SetupError::AlreadySetup,
                other => SetupError::Store(other),
            })?;
        let (session_id, _csrf) = self.session_store.create(user_id).await;
        Ok(session_id)
    }

    /// 会话 → 用户 id → enabled + role 的实时解析（design D5：角色不快照，
    /// 改角色即时生效、禁用/删号即刻失效）。返回 None = 会话无效、绑定的
    /// 用户已删/已禁用、或鉴权关闭——调用方（auth_guard，后续批次）按
    /// 未认证处理。admin 控制面会话（`user_id = 0`）不对应 webui 用户，
    /// 同样返回 None（其授权面由后续批次另行裁决）。
    pub async fn identity_for_session(&self, session_id: &str) -> Option<Identity> {
        if !self.enabled {
            return None;
        }
        let users = self.users.as_ref()?;
        let user_id = self.session_store.user_id_of(session_id).await?;
        if user_id == 0 {
            return None;
        }
        let user = match users.get(user_id) {
            Ok(user) => user,
            Err(_) => return None,
        };
        if !user.enabled {
            return None;
        }
        Some(Identity {
            user_id: user.id,
            username: user.username,
            role: user.role,
        })
    }

    /// 注销（移除会话）。
    pub async fn logout(&self, session_id: &str) {
        self.session_store.remove(session_id).await;
    }
}

// ─── 测试 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试统一用小迭代数（120k 在 debug 构建下单次哈希约秒级）。
    fn open_test_handle(dir: &Path) -> AuthHandle {
        AuthHandle::open_with_iterations(dir.join("auth.db"), 1000)
    }

    fn test_store(handle: &AuthHandle) -> &UserStore {
        handle.user_store().expect("测试句柄的用户库应在场")
    }

    #[tokio::test]
    async fn login_success_wrong_password_unknown_user_and_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let handle = open_test_handle(dir.path());
        assert!(handle.enabled());
        assert!(handle.needs_setup(), "零用户 = 首启设置页形态");

        // 零用户时任何登录都拒绝（不存在默认账户）。
        assert_eq!(
            handle.login("1.2.3.4", "alice", "password8").await,
            Err(LoginError::Invalid)
        );

        handle.setup_root("alice", "password8").await.unwrap();
        assert!(!handle.needs_setup(), "建 root 后不再是设置页形态");

        // 登录成功：会话有效且绑定该用户。
        let session = handle.login("1.2.3.4", "alice", "password8").await.unwrap();
        assert!(!session.is_empty());
        assert!(handle.session_store.validate(&session).await.is_ok());
        let identity = handle.identity_for_session(&session).await.unwrap();
        assert_eq!(identity.user_id, 1);
        assert_eq!(identity.username, "alice");
        assert_eq!(identity.role, Role::Root);

        // 密码错。
        assert_eq!(
            handle.login("1.2.3.4", "alice", "wrong-pass").await,
            Err(LoginError::Invalid)
        );
        // 用户名不存在（走哑哈希路径，同样 Invalid）。
        assert_eq!(
            handle.login("1.2.3.4", "bob", "password8").await,
            Err(LoginError::Invalid)
        );

        // 禁用后：既有会话立即失效（实时解析按无权限会话处理），登录被拒。
        // （用 member 账户验证——唯一 root 自身的禁用被 LastRoot 保护拦截，
        // 那是 user_store 层的职责，见其单测。）
        let carol_id = test_store(&handle)
            .create("carol", "password9", Role::Member)
            .unwrap()
            .id;
        let carol_session = handle.login("1.2.3.4", "carol", "password9").await.unwrap();
        test_store(&handle).set_enabled(carol_id, false).unwrap();
        assert!(handle.identity_for_session(&carol_session).await.is_none());
        assert_eq!(
            handle.login("1.2.3.4", "carol", "password9").await,
            Err(LoginError::AccountDisabled)
        );
        // 重新启用恢复（用户没删）。
        test_store(&handle).set_enabled(carol_id, true).unwrap();
        assert!(handle.identity_for_session(&carol_session).await.is_some());

        // 注销后会话失效。
        handle.logout(&session).await;
        assert!(handle.session_store.validate(&session).await.is_err());
    }

    /// 删除用户后其会话立即失效；改角色即时生效无需重登
    /// （spec「角色调整即时生效」/「会话绑定用户」）。
    #[tokio::test]
    async fn identity_resolves_role_and_death_in_realtime() {
        let dir = tempfile::tempdir().unwrap();
        let handle = open_test_handle(dir.path());
        handle.setup_root("root", "password8").await.unwrap();
        let uid = test_store(&handle)
            .create("carol", "password8", Role::Member)
            .unwrap()
            .id;
        let session = handle.login("1.2.3.4", "carol", "password8").await.unwrap();

        assert_eq!(
            handle.identity_for_session(&session).await.unwrap().role,
            Role::Member
        );

        // 降级 viewer：同一会话下一请求就按新角色解析。
        test_store(&handle).set_role(uid, Role::Viewer).unwrap();
        assert_eq!(
            handle.identity_for_session(&session).await.unwrap().role,
            Role::Viewer
        );

        // 删号：会话即使仍在 SessionStore 里也解析不出身份。
        test_store(&handle).delete(uid).unwrap();
        assert!(handle.identity_for_session(&session).await.is_none());
    }

    #[tokio::test]
    async fn disabled_handle_rejects_everything() {
        let handle = AuthHandle::disabled();
        assert!(!handle.enabled());
        assert!(handle.user_store().is_none());
        assert!(!handle.needs_setup());
        assert_eq!(
            handle.login("1.2.3.4", "alice", "password8").await,
            Err(LoginError::Disabled)
        );
        assert!(
            matches!(
                handle.setup_root("alice", "password8").await,
                Err(SetupError::Disabled)
            ),
            "disabled 句柄的 setup_root 应报 Disabled"
        );
        assert_eq!(handle.identity_for_session("whatever").await, None);
    }

    /// setup_root：首启建 root 成功并带会话；弱密码 400 态；非零用户 409 态；
    /// 弱密码不产生半初始化状态。
    #[tokio::test]
    async fn setup_root_success_weak_password_and_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let handle = open_test_handle(dir.path());

        // 弱密码（< 8）：拒绝且库保持零用户。
        assert!(
            matches!(
                handle.setup_root("root", "short").await,
                Err(SetupError::WeakPassword)
            ),
            "弱密码应报 WeakPassword"
        );
        assert!(handle.needs_setup(), "弱密码不得留下半初始化状态");

        // 空用户名：存储层拒绝（InvalidUsername 透传）。
        assert!(matches!(
            handle.setup_root("  ", "password8").await,
            Err(SetupError::Store(StoreError::InvalidUsername))
        ));

        // 成功：返回的会话有效、绑定 root。
        let session = handle.setup_root("root", "password8").await.unwrap();
        assert!(handle.session_store.validate(&session).await.is_ok());
        assert_eq!(handle.session_store.user_id_of(&session).await, Some(1));

        // 非零用户：一律 AlreadySetup（409）。
        assert!(
            matches!(
                handle.setup_root("second", "password8").await,
                Err(SetupError::AlreadySetup)
            ),
            "非零用户 setup 应报 AlreadySetup"
        );
        assert_eq!(test_store(&handle).count().unwrap(), 1);
    }

    /// 并发 setup 双请求（双句柄 = 双连接）：事务内零用户校验保证只成
    /// 一个 root，另一个 AlreadySetup（409）。
    #[tokio::test]
    async fn concurrent_setup_root_yields_exactly_one_root() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        let h1 = std::sync::Arc::new(AuthHandle::open_with_iterations(path.clone(), 1000));
        let h2 = std::sync::Arc::new(AuthHandle::open_with_iterations(path.clone(), 1000));

        let (r1, r2) = tokio::join!(h1.setup_root("a-root", "password8"), async {
            // 微错峰，让两个事务真挤同一窗口（同 tick 顺序执行也无妨——
            // 断言只认「恰好一个成功」）。
            tokio::task::yield_now().await;
            h2.setup_root("b-root", "password8").await
        });

        let outcomes = [r1, r2];
        assert_eq!(
            outcomes.iter().filter(|r| r.is_ok()).count(),
            1,
            "并发 setup 必须恰好成功一个：{outcomes:?}"
        );
        for r in &outcomes {
            if let Err(e) = r {
                assert!(
                    matches!(e, SetupError::AlreadySetup),
                    "失败方必须是 AlreadySetup：{e:?}"
                );
            }
        }
        let store = test_store(&h1);
        assert_eq!(store.count().unwrap(), 1);
        let users = store.list().unwrap();
        assert_eq!(users[0].role, Role::Root);
        assert!(matches!(users[0].username.as_str(), "a-root" | "b-root"));
    }

    #[tokio::test]
    async fn login_rate_limit_per_ip() {
        let dir = tempfile::tempdir().unwrap();
        let handle = open_test_handle(dir.path());
        handle.setup_root("alice", "password8").await.unwrap();

        for _ in 0..5 {
            let _ = handle.login("9.9.9.9", "alice", "wrong").await;
        }
        assert_eq!(
            handle.login("9.9.9.9", "alice", "password8").await,
            Err(LoginError::RateLimited),
            "连续失败后正确密码也被限速拦截"
        );
        // 其它 IP 不受影响。
        assert!(handle.login("8.8.8.8", "alice", "password8").await.is_ok());
    }

    /// 降级态（auth.db 打不开）：enabled 但一切登录/身份解析拒绝
    /// （design Risks：不静默降级为免鉴权）。
    #[tokio::test]
    async fn corrupted_db_stays_enabled_but_rejects_everything() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");
        // 用目录占位文件路径，让 SQLite 打不开。
        std::fs::create_dir_all(&path).unwrap();
        let handle = AuthHandle::open(path);
        assert!(handle.enabled(), "坏库不得降级为免鉴权");
        assert!(handle.user_store().is_none());
        assert!(!handle.needs_setup());
        assert_eq!(
            handle.login("1.2.3.4", "alice", "password8").await,
            Err(LoginError::Invalid)
        );
        assert_eq!(handle.identity_for_session("sid").await, None);
        assert!(matches!(
            handle.setup_root("alice", "password8").await,
            Err(SetupError::Unavailable)
        ));
    }
}
