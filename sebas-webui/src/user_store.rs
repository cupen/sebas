//! WebUI 用户库 auth.db：多用户账户的 SQLite 存储
//! （add-webui-multiuser-rbac 1.2/1.3，design D1）。
//!
//! # 存储形态
//!
//! 独立 SQLite 文件：默认 `~/.sebas/auth.db`，`SEBAS_WEBUI_AUTH_DB` 环境变量
//! 覆盖（沙箱/测试隔离，与 `SEBAS_STATE_DB` 同一模式）。连接配方复用
//! `sebas_state/db.rs` 的既有组合：rusqlite bundled、WAL、busy_timeout=5s、
//! foreign_keys=ON，schema 版本走 `PRAGMA user_version`（当前 = 1）。
//! 单 `Connection` 包在 `std::sync::Mutex` 里同步调用——鉴权流量是每请求
//! 一次主键查询（进程内 SQLite，µs 级），不值得 spawn_blocking 池。
//!
//! # 密码
//!
//! 只存 PBKDF2-HMAC-SHA256 哈希（随机盐 + 迭代次数随记录，便于将来上调），
//! 明文密码绝不落盘。哈希实现沿用原 `auth.rs` 的函数（已过 RFC 向量测试）。
//!
//! # 唯一性与保护
//!
//! - 用户名唯一**大小写不敏感**（`COLLATE NOCASE`：`Alice` 与 `alice` 同名）。
//! - 最后一个启用的 root 的删除/禁用/降级在存储层拒绝（[`StoreError::LastRoot`]），
//!   不依赖 handler 侧检查（design D6）。
//! - [`UserStore::setup_root`] 在事务内「数用户 → 建 root」原子完成，
//!   非零用户一律拒绝（[`StoreError::AlreadyInitialized`] → 409）。

use crate::rbac::Role;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// PBKDF2-HMAC-SHA256 迭代次数（与原 auth.rs 同值：OWASP 建议 SHA256 ≥ 600k，
/// 这里取折中值——debug 构建无优化下登录耗时仍在秒级；实际值随用户记录入库，
/// 可平滑上调）。
pub const PBKDF2_ITERATIONS: u32 = 120_000;

const SALT_LEN: usize = 16;
const HASH_LEN: usize = 32;
/// schema 版本（`PRAGMA user_version`）。
const SCHEMA_VERSION: i64 = 1;

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS users (
  id          INTEGER PRIMARY KEY,
  username    TEXT NOT NULL UNIQUE COLLATE NOCASE,
  role        TEXT NOT NULL CHECK (role IN ('root','admin','member','viewer')),
  iterations  INTEGER NOT NULL,
  salt_hex    TEXT NOT NULL,
  hash_hex    TEXT NOT NULL,
  enabled     INTEGER NOT NULL DEFAULT 1,
  created_at_unix INTEGER NOT NULL,
  updated_at_unix INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_users_enabled_root ON users(role, enabled);
";

/// 用户库路径：`SEBAS_WEBUI_AUTH_DB` 优先，否则 `~/.sebas/auth.db`。
pub fn default_auth_db() -> PathBuf {
    if let Ok(p) = std::env::var("SEBAS_WEBUI_AUTH_DB")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".sebas")
        .join("auth.db")
}

// ─── 密码哈希原语（自原 auth.rs 平移，实现不变） ─────────────────────────────

/// PBKDF2-HMAC-SHA256（RFC 8018），单块输出（32 字节，恰好是 SHA-256 摘要长）。
pub fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; HASH_LEN] {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

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

/// 一次密码哈希的入库三件套。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordHash {
    pub iterations: u32,
    pub salt: Vec<u8>,
    pub hash: Vec<u8>,
}

/// 哈希明文密码（随机盐 + 指定迭代次数）。
pub fn hash_password_with(password: &str, iterations: u32) -> PasswordHash {
    let salt = random_bytes(SALT_LEN);
    let hash = pbkdf2_hmac_sha256(password.as_bytes(), &salt, iterations);
    PasswordHash {
        iterations,
        salt,
        hash: hash.to_vec(),
    }
}

// ─── 记录形状 ───────────────────────────────────────────────────────────────

/// 完整用户记录（含哈希字段，仅服务端内部使用：登录验证、存储管理）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub id: i64,
    pub username: String,
    pub role: Role,
    pub iterations: u32,
    pub salt: Vec<u8>,
    pub hash: Vec<u8>,
    pub enabled: bool,
    pub created_at_unix: i64,
    pub updated_at_unix: i64,
}

impl UserRecord {
    /// 校验明文密码与本记录哈希（常量时间比较）。
    pub fn verify_password(&self, password: &str) -> bool {
        let candidate = pbkdf2_hmac_sha256(password.as_bytes(), &self.salt, self.iterations);
        constant_time_eq(&candidate, &self.hash)
    }
}

/// 对外用户信息（list / create 的返回形状）。类型上就不携带盐/哈希字段——
/// 用户管理 API 直接序列化它也不会泄漏凭据材料。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    pub role: Role,
    pub enabled: bool,
    pub created_at_unix: i64,
    pub updated_at_unix: i64,
}

// ─── 错误 ───────────────────────────────────────────────────────────────────

/// 用户库操作错误。`UsernameTaken` / `LastRoot` / `NotFound` 是 handler 侧
/// 语义化映射（400/409/404）的依据。
#[derive(Debug)]
pub enum StoreError {
    /// SQLite 层错误（打不开、损坏、SQL 失败）。
    Sql(rusqlite::Error),
    /// 文件系统错误（建目录失败等）。
    Io(std::io::Error),
    /// `user_version` 高于本二进制支持的版本（由更新版本的 sebas 创建）。
    IncompatibleVersion(i64),
    /// 用户名已存在（大小写不敏感判定）。
    UsernameTaken,
    /// 目标用户不存在。
    NotFound,
    /// 目标是最后一个启用的 root，删除/禁用/降级被拒。
    LastRoot,
    /// 用户库已有用户，setup 初始化被拒（→ 409）。
    AlreadyInitialized,
    /// 用户名为空（或全空白）。
    InvalidUsername,
    /// 记录字段损坏（角色词表外、hex 解不开——防御手工改库）。
    CorruptRecord(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sql(e) => write!(f, "auth.db 数据库错误: {e}"),
            StoreError::Io(e) => write!(f, "auth.db 文件错误: {e}"),
            StoreError::IncompatibleVersion(v) => write!(
                f,
                "auth.db schema 版本 {v} 高于本程序支持的 {SCHEMA_VERSION}（由更新版本的 sebas 创建？）"
            ),
            StoreError::UsernameTaken => write!(f, "用户名已存在（大小写不敏感）"),
            StoreError::NotFound => write!(f, "用户不存在"),
            StoreError::LastRoot => write!(f, "不能删除、禁用或降级最后一个启用的 root"),
            StoreError::AlreadyInitialized => write!(f, "用户库已有用户，无法再次初始化 root"),
            StoreError::InvalidUsername => write!(f, "用户名不能为空"),
            StoreError::CorruptRecord(detail) => write!(f, "auth.db 记录损坏: {detail}"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Sql(e) => Some(e),
            StoreError::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// 约束冲突 → UsernameTaken。users 表唯一的约束是 username UNIQUE 与
/// role CHECK，role 来自代码内 enum 恒合法，故约束冲突必是用户名撞车。
fn map_constraint(err: rusqlite::Error) -> StoreError {
    match &err {
        rusqlite::Error::SqliteFailure(ffi, _)
            if ffi.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            StoreError::UsernameTaken
        }
        _ => StoreError::Sql(err),
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 用户名归一：trim 后非空（其余原样入库；唯一性/查找由 NOCASE 列接管）。
fn validate_username(username: &str) -> Result<String, StoreError> {
    let trimmed = username.trim();
    if trimmed.is_empty() {
        return Err(StoreError::InvalidUsername);
    }
    Ok(trimmed.to_string())
}

// ─── UserStore ──────────────────────────────────────────────────────────────

/// auth.db 的同步句柄。内含单 `Connection`（`std::sync::Mutex`），全部方法
/// 阻塞调用方极短时间（主键查询级）；不要在持锁跨越 await 的场景使用。
#[derive(Debug)]
pub struct UserStore {
    conn: Mutex<Connection>,
    /// 新建/改密的默认迭代次数。生产 = [`PBKDF2_ITERATIONS`]；测试与工具
    /// （webui-passwd --iterations 等潜在用途）可用 [`Self::open_with_iterations`]
    /// 调低提速。
    default_iterations: u32,
}

impl UserStore {
    /// 打开（不存在则创建并初始化为零用户状态，不产生默认账户）。
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::open_with_iterations(path, PBKDF2_ITERATIONS)
    }

    /// 打开默认路径的库（[`default_auth_db`]）。
    pub fn open_default() -> Result<Self, StoreError> {
        Self::open(&default_auth_db())
    }

    /// 同 [`Self::open`] 但指定默认迭代次数（测试提速用）。
    pub fn open_with_iterations(path: &Path, default_iterations: u32) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(StoreError::Io)?;
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(StoreError::Sql)?;

        // 与 sebas_state/db.rs 同一配方（design D1）。
        conn.pragma_update(None, "journal_mode", "wal")
            .map_err(StoreError::Sql)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(StoreError::Sql)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(StoreError::Sql)?;

        // user_version 迁移：0（新库）→ 建表；更高版本 = 未来二进制所建，拒绝。
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(StoreError::Sql)?;
        if version > SCHEMA_VERSION {
            return Err(StoreError::IncompatibleVersion(version));
        }
        conn.execute_batch(SCHEMA_SQL).map_err(StoreError::Sql)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(StoreError::Sql)?;

        Ok(Self {
            conn: Mutex::new(conn),
            default_iterations,
        })
    }

    /// 新建/改密的默认 PBKDF2 迭代次数。
    pub fn default_iterations(&self) -> u32 {
        self.default_iterations
    }

    /// 当前用户总数（0 = 首启未初始化）。
    pub fn count(&self) -> Result<i64, StoreError> {
        self.with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
                .map_err(StoreError::Sql)
        })
    }

    /// 全量用户列表（按 id 升序）。**不含哈希字段**（[`UserInfo`] 形状保证）。
    pub fn list(&self) -> Result<Vec<UserInfo>, StoreError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, username, role, enabled, created_at_unix, updated_at_unix
                     FROM users ORDER BY id",
                )
                .map_err(StoreError::Sql)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(UserInfo {
                        id: row.get(0)?,
                        username: row.get(1)?,
                        role: role_col(row, 2)?,
                        enabled: row.get::<_, i64>(3)? != 0,
                        created_at_unix: row.get(4)?,
                        updated_at_unix: row.get(5)?,
                    })
                })
                .map_err(StoreError::Sql)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(StoreError::Sql)
        })
    }

    /// 按主键取完整记录（含哈希，登录/身份解析用）。
    pub fn get(&self, user_id: i64) -> Result<UserRecord, StoreError> {
        self.with_conn(|conn| fetch_record(conn, user_id))
    }

    /// 按用户名取完整记录（大小写不敏感）；不存在返回 `None`。
    pub fn get_by_username(&self, username: &str) -> Result<Option<UserRecord>, StoreError> {
        self.with_conn(|conn| {
            match conn.query_row(
                "SELECT id, username, role, iterations, salt_hex, hash_hex, enabled,
                        created_at_unix, updated_at_unix
                 FROM users WHERE username = ?1",
                [username],
                row_record,
            ) {
                Ok(record) => Ok(Some(record)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(StoreError::Sql(e)),
            }
        })
    }

    /// 创建用户（指定角色与初始密码，随机盐 + 默认迭代次数）。
    pub fn create(&self, username: &str, password: &str, role: Role) -> Result<UserInfo, StoreError> {
        self.create_with_iterations(username, password, role, self.default_iterations)
    }

    /// 同 [`Self::create`] 但指定迭代次数（测试提速用）。
    pub fn create_with_iterations(
        &self,
        username: &str,
        password: &str,
        role: Role,
        iterations: u32,
    ) -> Result<UserInfo, StoreError> {
        let username = validate_username(username)?;
        let ph = hash_password_with(password, iterations);
        let now = now_unix();
        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(StoreError::Sql)?;
            tx.execute(
                "INSERT INTO users
                     (username, role, iterations, salt_hex, hash_hex, enabled,
                      created_at_unix, updated_at_unix)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)",
                rusqlite::params![
                    username,
                    role.as_str(),
                    ph.iterations as i64,
                    hex::encode(&ph.salt),
                    hex::encode(&ph.hash),
                    now
                ],
            )
            .map_err(map_constraint)?;
            let id = tx.last_insert_rowid();
            tx.commit().map_err(StoreError::Sql)?;
            Ok(UserInfo {
                id,
                username,
                role,
                enabled: true,
                created_at_unix: now,
                updated_at_unix: now,
            })
        })
    }

    /// 首启建 root：事务内「数用户 → 插 root」原子完成（design D4）。非零用户
    /// 一律 [`StoreError::AlreadyInitialized`]（并发 setup 第二个请求撞这里）。
    ///
    /// 不做密码强度拦截——env/CLI 引导维持「warn 不拦截」姿态（短密码
    /// 引导 admin/admin 是测试环境的合法形态）；设置页路径的 ≥8 校验在
    /// `AuthHandle::setup_root`（400 层）。
    pub fn setup_root(&self, username: &str, password: &str) -> Result<i64, StoreError> {
        self.setup_root_with_iterations(username, password, self.default_iterations)
    }

    /// 同 [`Self::setup_root`] 但指定迭代次数（测试提速用）。
    pub fn setup_root_with_iterations(
        &self,
        username: &str,
        password: &str,
        iterations: u32,
    ) -> Result<i64, StoreError> {
        let username = validate_username(username)?;
        let ph = hash_password_with(password, iterations);
        let now = now_unix();
        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(StoreError::Sql)?;
            let count: i64 = tx
                .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
                .map_err(StoreError::Sql)?;
            if count > 0 {
                return Err(StoreError::AlreadyInitialized);
            }
            tx.execute(
                "INSERT INTO users
                     (username, role, iterations, salt_hex, hash_hex, enabled,
                      created_at_unix, updated_at_unix)
                 VALUES (?1, 'root', ?2, ?3, ?4, 1, ?5, ?5)",
                rusqlite::params![
                    username,
                    ph.iterations as i64,
                    hex::encode(&ph.salt),
                    hex::encode(&ph.hash),
                    now
                ],
            )
            .map_err(map_constraint)?;
            let id = tx.last_insert_rowid();
            tx.commit().map_err(StoreError::Sql)?;
            Ok(id)
        })
    }

    /// 重置密码（新随机盐 + 默认迭代次数；旧密码立即失效）。
    pub fn set_password(&self, user_id: i64, password: &str) -> Result<(), StoreError> {
        let ph = hash_password_with(password, self.default_iterations);
        self.with_conn(|conn| {
            let n = conn
                .execute(
                    "UPDATE users SET iterations = ?1, salt_hex = ?2, hash_hex = ?3,
                     updated_at_unix = ?4 WHERE id = ?5",
                    rusqlite::params![
                        ph.iterations as i64,
                        hex::encode(&ph.salt),
                        hex::encode(&ph.hash),
                        now_unix(),
                        user_id
                    ],
                )
                .map_err(StoreError::Sql)?;
            if n == 0 {
                Err(StoreError::NotFound)
            } else {
                Ok(())
            }
        })
    }

    /// 修改角色。目标是最后一个启用的 root 且在降级 → [`StoreError::LastRoot`]。
    pub fn set_role(&self, user_id: i64, role: Role) -> Result<(), StoreError> {
        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(StoreError::Sql)?;
            let target = fetch_record(&tx, user_id)?;
            if target.role == Role::Root
                && role != Role::Root
                && enabled_root_count(&tx)? <= 1
            {
                return Err(StoreError::LastRoot);
            }
            tx.execute(
                "UPDATE users SET role = ?1, updated_at_unix = ?2 WHERE id = ?3",
                rusqlite::params![role.as_str(), now_unix(), user_id],
            )
            .map_err(map_constraint)?;
            tx.commit().map_err(StoreError::Sql)?;
            Ok(())
        })
    }

    /// 启用/禁用。禁用最后一个启用的 root → [`StoreError::LastRoot`]。
    pub fn set_enabled(&self, user_id: i64, enabled: bool) -> Result<(), StoreError> {
        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(StoreError::Sql)?;
            let target = fetch_record(&tx, user_id)?;
            if !enabled
                && target.role == Role::Root
                && enabled_root_count(&tx)? <= 1
            {
                return Err(StoreError::LastRoot);
            }
            tx.execute(
                "UPDATE users SET enabled = ?1, updated_at_unix = ?2 WHERE id = ?3",
                rusqlite::params![enabled as i64, now_unix(), user_id],
            )
            .map_err(map_constraint)?;
            tx.commit().map_err(StoreError::Sql)?;
            Ok(())
        })
    }

    /// 删除用户。目标是最后一个启用的 root → [`StoreError::LastRoot`]。
    pub fn delete(&self, user_id: i64) -> Result<(), StoreError> {
        self.with_conn(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(StoreError::Sql)?;
            let target = fetch_record(&tx, user_id)?;
            if target.role == Role::Root && enabled_root_count(&tx)? <= 1 {
                return Err(StoreError::LastRoot);
            }
            tx.execute("DELETE FROM users WHERE id = ?1", [user_id])
                .map_err(StoreError::Sql)?;
            tx.commit().map_err(StoreError::Sql)?;
            Ok(())
        })
    }

    /// 与真实校验等代价的哑 PBKDF2（登录时用户名不存在也跑一次，抹平
    /// 「用户名错 vs 密码错」的时序差，防用户名枚举——design D5）。迭代数
    /// 取库默认值以对齐真实记录的验证成本。
    pub fn dummy_verify_delay(&self, password: &str) {
        const DUMMY_SALT: [u8; SALT_LEN] = [0x5e; SALT_LEN];
        let _ = pbkdf2_hmac_sha256(password.as_bytes(), &DUMMY_SALT, self.default_iterations);
    }

    fn with_conn<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut conn = self.conn.lock().expect("user store lock poisoned");
        f(&mut conn)
    }
}

/// 事务内按主键取记录（供 LastRoot 判定前的目标读取）。
fn fetch_record(conn: &Connection, user_id: i64) -> Result<UserRecord, StoreError> {
    conn.query_row(
        "SELECT id, username, role, iterations, salt_hex, hash_hex, enabled,
                created_at_unix, updated_at_unix
         FROM users WHERE id = ?1",
        [user_id],
        row_record,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound,
        other => StoreError::Sql(other),
    })
}

/// 当前启用的 root 数（LastRoot 保护的判定基准）。
fn enabled_root_count(conn: &Connection) -> Result<i64, StoreError> {
    conn.query_row(
        "SELECT COUNT(*) FROM users WHERE role = 'root' AND enabled = 1",
        [],
        |row| row.get(0),
    )
    .map_err(StoreError::Sql)
}

fn row_record(row: &rusqlite::Row) -> rusqlite::Result<UserRecord> {
    Ok(UserRecord {
        id: row.get(0)?,
        username: row.get(1)?,
        role: role_col(row, 2)?,
        iterations: row.get::<_, i64>(3)? as u32,
        salt: hex_col(row, 4)?,
        hash: hex_col(row, 5)?,
        enabled: row.get::<_, i64>(6)? != 0,
        created_at_unix: row.get(7)?,
        updated_at_unix: row.get(8)?,
    })
}

/// 读取角色列；词表外视为记录损坏（防御手工改库，不 panic）。
fn role_col(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<Role> {
    let raw: String = row.get(idx)?;
    raw.parse::<Role>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            Box::new(StoreError::CorruptRecord(e.to_string())),
        )
    })
}

fn hex_col(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<Vec<u8>> {
    let raw: String = row.get(idx)?;
    hex::decode(raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            Box::new(StoreError::CorruptRecord(format!("hex decode: {e}"))),
        )
    })
}

// ─── 测试 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试统一用小迭代数（120k 在 debug 构建下单次哈希约秒级）。
    fn open_test_store(dir: &Path, name: &str) -> UserStore {
        UserStore::open_with_iterations(&dir.join(name), 1000).expect("open test store")
    }

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
            hex::encode(v(
                "passwordPASSWORDpassword",
                "saltSALTsaltSALTsaltSALTsaltSALTsalt",
                4096
            )),
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

    // env 是进程全局的：default_auth_db 的覆盖用例共用一把锁串行。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn open_creates_db_idempotently_and_sets_user_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.db");

        // 首次打开：建文件、零用户（spec「首次访问建库」：不产生默认账户）。
        let store = UserStore::open_with_iterations(&path, 1000).unwrap();
        assert_eq!(store.count().unwrap(), 0);

        // 写入一个用户后再开第二次：schema 幂等、数据保持。
        store.create("alice", "password8", Role::Admin).unwrap();
        let reopened = UserStore::open_with_iterations(&path, 1000).unwrap();
        assert_eq!(reopened.count().unwrap(), 1, "重复建库不得丢数据");
        assert!(
            reopened.get_by_username("alice").unwrap().is_some(),
            "重复打开后既有用户仍在"
        );

        // user_version 钉在 1（design D1 的迁移锚点）。
        let raw = rusqlite::Connection::open(&path).unwrap();
        let version: i64 = raw
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        // WAL 生效（与 sebas_state 同配方）。
        let journal: String = raw
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal, "wal");
    }

    /// env 覆盖：`SEBAS_WEBUI_AUTH_DB` 优先于 `~/.sebas/auth.db`；
    /// 未设置时回落默认路径；空串视同未设置。
    #[test]
    fn env_var_overrides_default_auth_db_path() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let override_path = dir.path().join("custom").join("auth.db");

        // SAFETY: ENV_LOCK 保证进程内 env 访问串行。
        unsafe { std::env::set_var("SEBAS_WEBUI_AUTH_DB", &override_path) };
        assert_eq!(default_auth_db(), override_path);
        // open_default 经 env 落到覆盖路径（父目录自动创建）。
        let store = UserStore::open_default().unwrap();
        assert!(override_path.exists(), "覆盖路径应被实际创建");
        store.create("bob", "password8", Role::Member).unwrap();

        // 空串视同未设置。
        unsafe { std::env::set_var("SEBAS_WEBUI_AUTH_DB", "") };
        assert_eq!(
            default_auth_db().file_name().and_then(|s| s.to_str()),
            Some("auth.db"),
            "空 env 回落 ~/.sebas/auth.db"
        );

        // 未设置：同样回落默认（不触碰真实 home——只看文件名）。
        unsafe { std::env::remove_var("SEBAS_WEBUI_AUTH_DB") };
        assert_eq!(
            default_auth_db().file_name().and_then(|s| s.to_str()),
            Some("auth.db")
        );
    }

    #[test]
    fn username_unique_is_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_test_store(dir.path(), "case.db");
        store.create("alice", "password8", Role::Member).unwrap();

        // spec 场景「用户名大小写不敏感唯一」：Alice/A L I C E 都撞同名。
        for variant in ["Alice", "ALICE", "alice", "aLiCe"] {
            let err = store
                .create(variant, "password8", Role::Viewer)
                .unwrap_err();
            assert!(
                matches!(err, StoreError::UsernameTaken),
                "创建 {variant} 应报 UsernameTaken，实际 {err:?}"
            );
        }
        assert_eq!(store.count().unwrap(), 1);

        // 查找同样大小写不敏感。
        let found = store.get_by_username("ALICE").unwrap().expect("命中");
        assert_eq!(found.username, "alice", "返回入库原形，不回改写形");
        assert!(store.get_by_username("carol").unwrap().is_none());
    }

    /// spec 场景「最后一个 root 受保护」：唯一启用 root 的删/禁/降级全部
    /// 在存储层拒绝（design D6，不依赖 handler 侧检查）。
    #[test]
    fn last_enabled_root_is_protected_from_delete_disable_and_demotion() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_test_store(dir.path(), "lastroot.db");
        let root_id = store.setup_root("root", "password8").unwrap();
        let admin_id = store
            .create("admin", "password8", Role::Admin)
            .unwrap()
            .id;

        // 删除 / 禁用 / 降级 → LastRoot，root 保持原状。
        assert!(matches!(store.delete(root_id), Err(StoreError::LastRoot)));
        assert!(matches!(
            store.set_enabled(root_id, false),
            Err(StoreError::LastRoot)
        ));
        assert!(matches!(
            store.set_role(root_id, Role::Admin),
            Err(StoreError::LastRoot)
        ));
        let root = store.get(root_id).unwrap();
        assert_eq!(root.role, Role::Root);
        assert!(root.enabled);

        // 非 root 用户不受保护：admin 可删。
        assert!(store.delete(admin_id).is_ok());

        // 第二个 root 就位后，保护解除：任一 root 的删/禁/降级都放行
        // （只要仍有启用 root 存活）。
        let second_id = store.create("root2", "password8", Role::Root).unwrap().id;
        assert!(store.set_role(root_id, Role::Admin).is_ok());
        assert!(store.delete(root_id).is_ok());
        // 此时 root2 又成为唯一 root：保护重新生效。
        assert!(matches!(store.delete(second_id), Err(StoreError::LastRoot)));

        // 目标不存在 → NotFound（而非 LastRoot）。
        assert!(matches!(store.delete(9999), Err(StoreError::NotFound)));
        assert!(matches!(
            store.set_enabled(9999, false),
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            store.set_role(9999, Role::Viewer),
            Err(StoreError::NotFound)
        ));
    }

    /// list 输出形状钉死：只含公开字段，绝不出现盐/哈希材料。
    #[test]
    fn list_output_never_carries_hash_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_test_store(dir.path(), "list.db");
        store.create("alice", "password8", Role::Admin).unwrap();
        store.create("bob", "password9", Role::Viewer).unwrap();

        let users = store.list().unwrap();
        assert_eq!(users.len(), 2);
        for user in users {
            let json = serde_json::to_value(&user).unwrap();
            let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
            assert_eq!(
                keys,
                ["id", "username", "role", "enabled", "created_at_unix", "updated_at_unix"],
                "list 字段集被改动：{json}"
            );
            let raw = serde_json::to_string(&user).unwrap();
            for leak in ["salt", "hash", "iterations"] {
                assert!(!raw.contains(leak), "list 输出泄漏 {leak}: {raw}");
            }
        }
    }

    /// spec 场景「密码不落明文」：建户/改密后，db 文件字节里检索不到明文，
    /// 且新哈希可验证、旧密码失效。
    #[test]
    fn plaintext_password_never_hits_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.db");
        let store = UserStore::open_with_iterations(&path, 1000).unwrap();
        let secret = "super-secret-pw-42";
        let id = store.create("alice", secret, Role::Member).unwrap().id;

        let raw = std::fs::read(&path).unwrap();
        let raw_text = String::from_utf8_lossy(&raw).to_string();
        assert!(
            !raw_text.contains(secret),
            "auth.db 不得包含明文密码（建户）"
        );
        assert!(store.get(id).unwrap().verify_password(secret));

        // 改密：明文同样不落盘；旧密码失效、新密码生效。
        let new_secret = "rotated-secret-pw-99";
        store.set_password(id, new_secret).unwrap();
        let raw = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).to_string();
        assert!(!raw.contains(secret), "旧密码明文也不应在库");
        assert!(!raw.contains(new_secret), "auth.db 不得包含明文密码（改密）");
        let record = store.get(id).unwrap();
        assert!(!record.verify_password(secret));
        assert!(record.verify_password(new_secret));

        // 随机盐：同密码两次建户哈希不同。
        let a = store.create("u1", "same-password", Role::Viewer).unwrap().id;
        let b = store.create("u2", "same-password", Role::Viewer).unwrap().id;
        assert_ne!(store.get(a).unwrap().hash, store.get(b).unwrap().hash);
    }

    #[test]
    fn crud_round_trip_and_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_test_store(dir.path(), "crud.db");

        let info = store.create("carol", "password8", Role::Member).unwrap();
        assert_eq!(info.username, "carol");
        assert_eq!(info.role, Role::Member);
        assert!(info.enabled);

        // get / get_by_username 一致。
        let record = store.get(info.id).unwrap();
        assert_eq!(record.username, "carol");
        assert!(record.verify_password("password8"));
        assert_eq!(
            store.get_by_username("carol").unwrap().unwrap().id,
            info.id
        );

        // set_role。
        store.set_role(info.id, Role::Viewer).unwrap();
        assert_eq!(store.get(info.id).unwrap().role, Role::Viewer);

        // set_enabled：禁用后 enabled=false，再启用恢复。
        store.set_enabled(info.id, false).unwrap();
        assert!(!store.get(info.id).unwrap().enabled);
        store.set_enabled(info.id, true).unwrap();
        assert!(store.get(info.id).unwrap().enabled);

        // set_password 走随机新盐（上面 plain 用例已验语义，这里只验错误面）。
        assert!(matches!(
            store.set_password(9999, "password8"),
            Err(StoreError::NotFound)
        ));

        // delete：删除后所有读取路径 NotFound。
        store.delete(info.id).unwrap();
        assert!(matches!(store.get(info.id), Err(StoreError::NotFound)));
        assert!(store.get_by_username("carol").unwrap().is_none());
        assert_eq!(store.count().unwrap(), 0);
        assert!(store.list().unwrap().is_empty());
    }

    /// setup_root：首次成功建 root；重复调用 AlreadyInitialized；空用户名
    /// InvalidUsername；同密码不同次哈希不同（随机盐）。
    #[test]
    fn setup_root_once_then_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_test_store(dir.path(), "setup.db");

        let root_id = store.setup_root("root", "password8").unwrap();
        let record = store.get(root_id).unwrap();
        assert_eq!(record.role, Role::Root);
        assert!(record.enabled);
        assert!(record.verify_password("password8"));

        // 非零用户：任何后续 setup 一律拒绝（spec「已有用户后 setup 拒绝」）。
        assert!(matches!(
            store.setup_root("other", "password8"),
            Err(StoreError::AlreadyInitialized)
        ));
        assert_eq!(store.count().unwrap(), 1);

        // 空用户名在零用户侧也拒绝（不产生半初始化状态）。
        let empty_dir = tempfile::tempdir().unwrap();
        let fresh = open_test_store(empty_dir.path(), "fresh.db");
        assert!(matches!(
            fresh.setup_root("   ", "password8"),
            Err(StoreError::InvalidUsername)
        ));
        assert_eq!(fresh.count().unwrap(), 0);
        // create 同样拒绝空用户名。
        assert!(matches!(
            fresh.create("", "password8", Role::Viewer),
            Err(StoreError::InvalidUsername)
        ));
    }

    /// 两个独立连接（模拟并发 setup 双请求）在同一 db 上抢注：事务内
    /// 零用户校验 + IMMEDIATE 写锁保证只有一个 root 成形。双线程真实
    /// 并发（串行调用测不出事务窗口）。
    #[test]
    fn concurrent_setup_root_on_shared_db_yields_single_root() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("race.db");
        // 两个连接先在主线程串行打开（建库/建 schema 不是本测的竞争点，
        // 并发首次建库会让 busy 竞争掩盖被测行为）。
        let stores = [
            UserStore::open_with_iterations(&path, 1000).unwrap(),
            UserStore::open_with_iterations(&path, 1000).unwrap(),
        ];

        let outcomes: Vec<Result<i64, StoreError>> = std::thread::scope(|scope| {
            let handles: Vec<_> = stores
                .into_iter()
                .enumerate()
                .map(|(i, store)| {
                    scope.spawn(move || store.setup_root(&format!("r{i}"), "password8"))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        let winners = outcomes.iter().filter(|r| r.is_ok()).count();
        assert_eq!(winners, 1, "并发 setup 必须恰好成功一个：{outcomes:?}");
        for r in &outcomes {
            match r {
                Ok(_) => {}
                Err(e) => assert!(matches!(e, StoreError::AlreadyInitialized), "{e:?}"),
            }
        }
        let check = UserStore::open_with_iterations(&path, 1000).unwrap();
        assert_eq!(check.count().unwrap(), 1);
        // 成功者必须真的是 root 且密码可验。
        let winner_id = *outcomes.iter().find_map(|r| r.as_ref().ok()).unwrap();
        let record = check.get(winner_id).unwrap();
        assert_eq!(record.role, Role::Root);
        assert!(record.verify_password("password8"));
    }

    /// 打不开的库（父目录被文件占位）→ 错误而非 panic；损坏检测由
    /// IncompatibleVersion 分支覆盖未来版本号。
    #[test]
    fn incompatible_future_version_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.db");
        {
            let raw = rusqlite::Connection::open(&path).unwrap();
            raw.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
                .unwrap();
        }
        let err = UserStore::open_with_iterations(&path, 1000).unwrap_err();
        assert!(
            matches!(err, StoreError::IncompatibleVersion(v) if v == SCHEMA_VERSION + 1),
            "{err:?}"
        );
    }
}
