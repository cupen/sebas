//! WebUI 登录鉴权：单账户「用户名 / 密码」。
//!
//! # 凭据存储
//!
//! 凭据落盘为 JSON（默认 `~/.sebas/webui-auth.json`，`SEBAS_WEBUI_AUTH_FILE`
//! 覆盖）：密码绝不存明文，只存 PBKDF2-HMAC-SHA256（随机盐 + 迭代次数，
//! 迭代次数一并入库以便将来上调）。
//!
//! # 生命周期
//!
//! - 初始化 / 修改：`sebas webui-passwd` 重写凭据文件；运行中的 webui 进程
//!   通过 mtime 探测热重载，改密后无需重启。
//! - 引导：`SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD` 环境变量在凭据文件
//!   缺失时自动建户（容器/公网部署用）。
//! - 未配置凭据 = 鉴权关闭（本地 loopback 开发零摩擦）；而**非 loopback
//!   bind 只有在凭据存在时才被放行**（见 `webui_cmd`），保证公网部署必带鉴权。
//!
//! 会话与限速复用 [`crate::admin_auth::SessionStore`]（24h 不活动 TTL、
//! per-IP 登录限速）。

use crate::admin_auth::SessionStore;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// PBKDF2-HMAC-SHA256 迭代次数（OWASP 建议 SHA256 ≥ 600k，这里取折中值：
/// debug 构建无优化下登录耗时仍在秒级；实际值随凭据入库，可平滑上调）。
pub const PBKDF2_ITERATIONS: u32 = 120_000;
const SALT_LEN: usize = 16;
const HASH_LEN: usize = 32;
/// 凭据文件格式版本（将来迁移用）。
const FORMAT_VERSION: u32 = 1;

/// WebUI 会话 cookie 名（HttpOnly + SameSite=Lax）。
pub const SESSION_COOKIE_NAME: &str = "sebas_webui_session";

/// 凭据文件路径：`SEBAS_WEBUI_AUTH_FILE` 优先，否则 `~/.sebas/webui-auth.json`。
pub fn default_auth_file() -> PathBuf {
    if let Ok(p) = std::env::var("SEBAS_WEBUI_AUTH_FILE")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".sebas")
        .join("webui-auth.json")
}

/// PBKDF2-HMAC-SHA256（RFC 8018），单块输出（32 字节，恰好是 SHA-256 摘要长）。
pub fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; HASH_LEN] {
    let mut mac = Hmac::<Sha256>::new_from_slice(password).expect("HMAC accepts any key length");
    // U_1 = PRF(P, S || INT(i))，i = 1（单块输出，无需多块拼接）。
    mac.update(salt);
    mac.update(&1u32.to_be_bytes());
    let mut u = mac.finalize().into_bytes();
    let mut block = [0u8; HASH_LEN];
    block.copy_from_slice(&u);
    for _ in 1..iterations.max(1) {
        mac = Hmac::<Sha256>::new_from_slice(password).expect("HMAC accepts any key length");
        mac.update(&u);
        u = mac.finalize().into_bytes();
        for (b, ui) in block.iter_mut().zip(u.iter()) {
            *b ^= ui;
        }
    }
    block
}

/// 常量时间比较，避免逐字节短路泄漏前缀匹配位置。
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// OS CSPRNG 随机字节（盐、会话 token 等安全用途）。
pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    getrandom::fill(&mut buf).expect("OS CSPRNG unavailable");
    buf
}

/// 已加载的凭据（内存态）。
#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: String,
    pub iterations: u32,
    pub salt: Vec<u8>,
    pub hash: Vec<u8>,
}

impl Credentials {
    /// 从明文密码建凭据（随机盐，默认迭代次数）。
    pub fn new(username: &str, password: &str) -> Self {
        Self::with_iterations(username, password, PBKDF2_ITERATIONS)
    }

    /// 指定迭代次数（测试用小值提速；生产走 [`Self::new`]）。
    pub fn with_iterations(username: &str, password: &str, iterations: u32) -> Self {
        let salt = random_bytes(SALT_LEN);
        let hash = pbkdf2_hmac_sha256(password.as_bytes(), &salt, iterations);
        Self {
            username: username.to_string(),
            iterations,
            salt,
            hash: hash.to_vec(),
        }
    }

    /// 校验用户名 + 密码。用户名不匹配与密码错误同样返回 false（单账户
    /// 部署下不向攻击者区分「用户名错」与「密码错」）。
    pub fn verify(&self, username: &str, password: &str) -> bool {
        if self.username != username {
            return false;
        }
        let candidate = pbkdf2_hmac_sha256(password.as_bytes(), &self.salt, self.iterations);
        constant_time_eq(&candidate, &self.hash)
    }
}

// ─── 凭据文件（JSON 落盘） ─────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct CredentialsFile {
    version: u32,
    username: String,
    iterations: u32,
    salt_hex: String,
    hash_hex: String,
    created_at_unix: u64,
    updated_at_unix: u64,
}

impl CredentialsFile {
    fn from_credentials(c: &Credentials, created_at_unix: u64) -> Self {
        Self {
            version: FORMAT_VERSION,
            username: c.username.clone(),
            iterations: c.iterations,
            salt_hex: hex::encode(&c.salt),
            hash_hex: hex::encode(&c.hash),
            created_at_unix,
            updated_at_unix: now_unix(),
        }
    }

    fn to_credentials(&self) -> Option<Credentials> {
        if self.version != FORMAT_VERSION {
            return None;
        }
        let salt = hex::decode(&self.salt_hex).ok()?;
        let hash = hex::decode(&self.hash_hex).ok()?;
        if salt.is_empty() || hash.is_empty() || self.username.is_empty() {
            return None;
        }
        Some(Credentials {
            username: self.username.clone(),
            iterations: self.iterations,
            salt,
            hash,
        })
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 读取凭据文件；文件不存在返回 `Ok(None)`（= 鉴权关闭），存在但损坏返回
/// `Err`（调用方应拒绝启动/拒绝登录，而不是静默降级为无鉴权）。
pub fn load_credentials(path: &Path) -> Result<Option<Credentials>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("read {}: {e}", path.display())),
    };
    let file: CredentialsFile = serde_json::from_str(&raw)
        .map_err(|e| format!("parse {}: {e}", path.display()))?;
    file.to_credentials()
        .map(Some)
        .ok_or_else(|| format!("unsupported credentials format in {}", path.display()))
}

/// 写凭据文件（tmp + rename 原子替换）。目录不存在则创建。
pub fn store_credentials(path: &Path, credentials: &Credentials) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }
    // created_at 尽量保留旧文件里的初建时间。
    let created_at_unix = load_credentials_raw_created(path).unwrap_or_else(|_| now_unix());
    let file = CredentialsFile::from_credentials(credentials, created_at_unix);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&file).expect("serializes"))
        .map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename into {}: {e}", path.display()))?;
    Ok(())
}

fn load_credentials_raw_created(path: &Path) -> Result<u64, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let file: CredentialsFile =
        serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))?;
    Ok(file.created_at_unix)
}

// ─── AuthHandle（服务端共享态） ────────────────────────────────────────────

/// 登录失败原因（HTTP 语义由 handler 决定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginError {
    /// 服务端未配置凭据，鉴权关闭。
    Disabled,
    /// 用户名或密码错误。
    Invalid,
    /// 该来源 IP 触发登录限速。
    RateLimited,
}

/// 服务端鉴权共享态：凭据（mtime 热重载）+ 会话存储。
pub struct AuthHandle {
    path: PathBuf,
    inner: RwLock<AuthInner>,
    pub session_store: SessionStore,
}

struct AuthInner {
    credentials: Option<Credentials>,
    mtime: Option<SystemTime>,
}

impl AuthHandle {
    /// 无凭据的关闭态（测试与未接线的部署路径用）。
    pub fn disabled() -> Self {
        Self {
            path: PathBuf::new(),
            inner: RwLock::new(AuthInner {
                credentials: None,
                mtime: None,
            }),
            session_store: SessionStore::new(),
        }
    }

    /// 打开凭据文件并加载。文件缺失/损坏都不 panic：缺失 = 鉴权关闭，
    /// 损坏 = 凭据视为 None 但 `degraded` 记录在案（登录一律拒绝）。
    pub fn open(path: PathBuf) -> Self {
        let mtime = file_mtime(&path);
        let credentials = match load_credentials(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "webui 凭据文件损坏；登录将拒绝");
                // 占位凭据：verify 永远 false（用户名不可能匹配到）。
                Some(Credentials {
                    username: "\u{0}corrupted".into(),
                    iterations: 1,
                    salt: vec![0],
                    hash: vec![0],
                })
            }
        };
        Self {
            path,
            inner: RwLock::new(AuthInner {
                credentials,
                mtime,
            }),
            session_store: SessionStore::new(),
        }
    }

    /// 凭据文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 鉴权是否开启（凭据存在）。每次调用先探测 mtime 热重载，让
    /// `sebas webui-passwd` 的修改无需重启即生效。
    pub fn enabled(&self) -> bool {
        self.reload_if_changed();
        self.inner.read().expect("auth lock").credentials.is_some()
    }

    /// 当前账户名（未配置时 None）。
    pub fn username(&self) -> Option<String> {
        self.reload_if_changed();
        self.inner
            .read()
            .expect("auth lock")
            .credentials
            .as_ref()
            .map(|c| c.username.clone())
    }

    /// 校验并创建会话，返回会话 cookie 值。限速按来源 IP（与 admin 登录
    /// 共用 SessionStore 的限速逻辑）。
    pub async fn login(
        &self,
        client_ip: &str,
        username: &str,
        password: &str,
    ) -> Result<String, LoginError> {
        self.reload_if_changed();
        let credentials = self
            .inner
            .read()
            .expect("auth lock")
            .credentials
            .clone();
        let Some(credentials) = credentials else {
            return Err(LoginError::Disabled);
        };
        if !self.session_store.check_rate_limit(client_ip).await {
            return Err(LoginError::RateLimited);
        }
        if !credentials.verify(username, password) {
            return Err(LoginError::Invalid);
        }
        self.session_store.reset_rate_limit(client_ip).await;
        let (session_id, _csrf) = self.session_store.create().await;
        Ok(session_id)
    }

    /// 注销（移除会话）。
    pub async fn logout(&self, session_id: &str) {
        self.session_store.remove(session_id).await;
    }

    /// mtime 变了才重读文件；`webui-passwd` 改密后无需重启。
    fn reload_if_changed(&self) {
        let current_mtime = file_mtime(&self.path);
        let needs_reload = {
            let inner = self.inner.read().expect("auth lock");
            inner.mtime != current_mtime
        };
        if !needs_reload {
            return;
        }
        let credentials = match load_credentials(&self.path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(path = %self.path.display(), error = %e, "webui 凭据文件重读失败；沿用旧凭据");
                // 保持旧凭据但更新 mtime，避免每次请求都重读失败。
                None
            }
        };
        let mut inner = self.inner.write().expect("auth lock");
        if credentials.is_some() || file_mtime(&self.path).is_none() {
            // 文件被删除 → 显式关闭鉴权；重读失败 → 保留旧凭据。
            if credentials.is_some() {
                inner.credentials = credentials;
            } else if file_mtime(&self.path).is_none() {
                inner.credentials = None;
            }
        }
        inner.mtime = current_mtime;
    }
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

// ─── 测试 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_matches_rfc_vectors() {
        // PBKDF2-HMAC-SHA256 公开测试向量（draft-josefsson-pbkdf2-test-vectors，
        // 取 dkLen=32 变体——本实现输出单块 32 字节）。
        let v = |pass: &str, salt: &str, iters: u32| {
            pbkdf2_hmac_sha256(pass.as_bytes(), salt.as_bytes(), iters)
        };
        assert_eq!(
            hex::encode(v("password", "salt", 1)),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        assert_eq!(
            hex::encode(v("password", "salt", 2)),
            "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
        );
        assert_eq!(
            hex::encode(v("password", "salt", 4096)),
            "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a"
        );
        assert_eq!(
            hex::encode(v("passwordPASSWORDpassword", "saltSALTsaltSALTsaltSALTsaltSALTsalt", 4096)),
            "348c89dbcbd32b2f32d814b8116e84cf2b17347ebc1800181c4e2a1fb8dd53e1"
        );
    }

    #[test]
    fn constant_time_eq_behaves() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn credentials_verify_round_trip() {
        let c = Credentials::with_iterations("alice", "s3cret", 1000);
        assert!(c.verify("alice", "s3cret"));
        assert!(!c.verify("alice", "wrong"));
        assert!(!c.verify("bob", "s3cret"));
        // 盐随机：同密码两次建户哈希不同。
        let c2 = Credentials::with_iterations("alice", "s3cret", 1000);
        assert_ne!(c.hash, c2.hash, "随机盐必须让同密码产生不同哈希");
    }

    #[test]
    fn credentials_file_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webui-auth.json");
        assert!(load_credentials(&path).unwrap().is_none(), "缺文件 = 未配置");

        let c = Credentials::with_iterations("carol", "pw123456", 1000);
        store_credentials(&path, &c).unwrap();
        let loaded = load_credentials(&path).unwrap().expect("written");
        assert_eq!(loaded.username, "carol");
        assert!(loaded.verify("carol", "pw123456"));
        assert!(!loaded.verify("carol", "nope"));

        // 改密：覆盖写后旧密码失效。
        let c2 = Credentials::with_iterations("carol", "new-pass", 1000);
        store_credentials(&path, &c2).unwrap();
        let loaded = load_credentials(&path).unwrap().unwrap();
        assert!(!loaded.verify("carol", "pw123456"));
        assert!(loaded.verify("carol", "new-pass"));
    }

    #[test]
    fn credentials_file_corruption_is_an_error_not_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webui-auth.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(load_credentials(&path).is_err(), "损坏文件必须报错而非视为未配置");
    }

    #[tokio::test]
    async fn auth_handle_login_flow_and_hot_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webui-auth.json");

        // 未配置 → 鉴权关闭。
        let handle = AuthHandle::open(path.clone());
        assert!(!handle.enabled());
        assert_eq!(
            handle.login("1.2.3.4", "alice", "pw").await,
            Err(LoginError::Disabled)
        );

        // 建户（小迭代数保证测试速度）→ 立即生效（mtime 热重载）。
        store_credentials(&path, &Credentials::with_iterations("alice", "pw", 1000)).unwrap();
        assert!(handle.enabled());
        assert_eq!(handle.username().as_deref(), Some("alice"));

        let bad = handle.login("1.2.3.4", "alice", "wrong").await;
        assert_eq!(bad, Err(LoginError::Invalid));
        let bad = handle.login("1.2.3.4", "bob", "pw").await;
        assert_eq!(bad, Err(LoginError::Invalid));

        let session = handle.login("1.2.3.4", "alice", "pw").await.unwrap();
        assert!(!session.is_empty());
        assert!(handle.session_store.validate(&session).await.is_ok());

        // 改密 → 旧会话仍有效（会话独立于凭据），新密码生效、旧密码失效。
        store_credentials(&path, &Credentials::with_iterations("alice", "newpw", 1000)).unwrap();
        assert_eq!(
            handle.login("1.2.3.4", "alice", "pw").await,
            Err(LoginError::Invalid)
        );
        assert!(handle.login("1.2.3.4", "alice", "newpw").await.is_ok());
        assert!(handle.session_store.validate(&session).await.is_ok());

        // 注销后会话失效。
        handle.logout(&session).await;
        assert!(handle.session_store.validate(&session).await.is_err());
    }

    #[tokio::test]
    async fn auth_handle_file_deletion_disables_auth() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webui-auth.json");
        store_credentials(&path, &Credentials::with_iterations("alice", "pw", 1000)).unwrap();
        let handle = AuthHandle::open(path.clone());
        assert!(handle.enabled());

        std::fs::remove_file(&path).unwrap();
        assert!(!handle.enabled(), "凭据文件删除必须即时关闭鉴权");
        assert_eq!(
            handle.login("1.2.3.4", "alice", "pw").await,
            Err(LoginError::Disabled)
        );
    }

    #[tokio::test]
    async fn auth_handle_corrupted_file_still_rejects_login() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webui-auth.json");
        std::fs::write(&path, "{broken").unwrap();
        let handle = AuthHandle::open(path);
        // 损坏文件不能静默变成「鉴权关闭」；enabled() 语义上仍算有凭据面，
        // 但任何登录都被拒绝。
        assert!(!handle.login("1.2.3.4", "alice", "pw").await.is_ok());
    }

    #[tokio::test]
    async fn login_rate_limit_per_ip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webui-auth.json");
        store_credentials(&path, &Credentials::with_iterations("alice", "pw", 1000)).unwrap();
        let handle = AuthHandle::open(path);

        for _ in 0..5 {
            let _ = handle.login("9.9.9.9", "alice", "wrong").await;
        }
        assert_eq!(
            handle.login("9.9.9.9", "alice", "pw").await,
            Err(LoginError::RateLimited),
            "连续失败后正确密码也被限速拦截"
        );
        // 其它 IP 不受影响。
        assert!(handle.login("8.8.8.8", "alice", "pw").await.is_ok());
    }
}
