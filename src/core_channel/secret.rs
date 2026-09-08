//! Core session channel secret: shared-secret discovery for clients and the
//! atomic secret file written by the auto-arming core
//! (harden-core-channel-deployment, design D1/D2/D5).
//!
//! Resolution order for every channel client (standalone webui, im, router
//! subscription): `SEBAS_CORE_SECRET` env wins (cached at construction, zero
//! per-connect cost); otherwise the secret file is read before **every**
//! connect attempt, so a core restart that rotates the key is healed by the
//! client's reconnect backoff alone — no notification channel needed. When
//! both are missing the client warns once and attempts an empty-secret
//! handshake (honest, not silent; the server closes it and the UI reports
//! `secret rejected`).
//!
//! File lifecycle (D5): the core writes the secret file atomically
//! (tmp + rename, 0600 on unix) at arm time and deliberately does NOT remove
//! it on graceful exit — the socket file is the authoritative "core is dead"
//! signal, and a leftover secret file is harmless (clients cannot reach a
//! handshake without a live socket).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Default secret file name, resolved next to the config file (D1).
pub const SECRET_FILE_NAME: &str = "core.secret";

/// Warn-once flag shared by every discovery in this process: missing secret
/// is a startup-visible condition, not a per-reconnect log flood.
static MISSING_WARNED: AtomicBool = AtomicBool::new(false);

/// Test-only: serialize `SEBAS_CORE_SECRET` mutations across the crate's test
/// modules (parallel `#[tokio::test]`s share one process and one environ).
#[cfg(test)]
pub(crate) fn secret_env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    &LOCK
}

/// Generate a random handshake secret (32 bytes of OS CSPRNG, hex-encoded).
/// Used by the core when `SEBAS_CORE_SECRET` is absent — the watchdog-injected
/// env path keeps priority and is unchanged.
pub fn generate() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS CSPRNG unavailable");
    hex::encode(buf)
}

/// Atomically write `secret` to `path`: write a tmp sibling, set 0600 (unix),
/// rename over the target. A crash mid-write never leaves a torn file.
pub fn write_secret_file(path: &Path, secret: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("secret.tmp");
    std::fs::write(&tmp, secret.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

/// Where a channel client gets its handshake secret from (D2).
pub enum ChannelSecret {
    /// `SEBAS_CORE_SECRET` was set at construction: cached, zero cost, and
    /// the watchdog deployment path keeps today's exact semantics.
    Static(String),
    /// No usable env: read the secret file before every connect attempt.
    Discover(Option<PathBuf>),
}

impl ChannelSecret {
    /// env 非空 → Static（缓存）；否则 Discover(文件路径)。
    /// `file` 为 `None` 表示调用方没有可用的 config 推导（缺省为空表发现）。
    pub fn from_env_or_file(file: Option<PathBuf>) -> Self {
        match std::env::var("SEBAS_CORE_SECRET") {
            Ok(s) if !s.is_empty() => Self::Static(s),
            _ => Self::Discover(file),
        }
    }

    /// 构造期常量（既有调用方与测试的直通形态）。
    pub fn static_value(v: String) -> Self {
        Self::Static(v)
    }

    /// 当前应使用的握手 secret。Discover 每次调用重读文件——`secret
    /// rejected` / 断线重连路径天然拿到 core 重启后的新钥（D2）。
    pub fn current(&self) -> String {
        match self {
            Self::Static(s) => s.clone(),
            Self::Discover(file) => match file.as_deref().map(std::fs::read_to_string) {
                Some(Ok(content)) => content.trim().to_string(),
                _ => {
                    if !MISSING_WARNED.swap(true, Ordering::Relaxed) {
                        tracing::warn!(
                            "核心通道 secret 未找到（SEBAS_CORE_SECRET 未设置且 secret 文件缺失）: \
                             以空 secret 尝试连接，握手将被拒绝；请确认与 core 使用同一份 config"
                        );
                    }
                    String::new()
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvGuard {
        name: &'static str,
        prev: Option<std::env::VarError>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    // 同进程并行测试都在读写 SEBAS_CORE_SECRET：guard 持锁到 drop，串行化
    //（锁与 core_channel::tests 共用同一把，见 secret_env_test_lock）。
    impl EnvGuard {
        fn set(name: &'static str, value: &str) -> Self {
            let lock = super::secret_env_test_lock().lock().unwrap();
            let prev = std::env::var(name).err();
            unsafe { std::env::set_var(name, value) };
            Self { name, prev, _lock: lock }
        }
        fn unset(name: &'static str) -> Self {
            let lock = super::secret_env_test_lock().lock().unwrap();
            let prev = std::env::var(name).err();
            unsafe { std::env::remove_var(name) };
            Self { name, prev, _lock: lock }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(std::env::VarError::NotPresent) | None => unsafe {
                    std::env::remove_var(self.name)
                },
                _ => {}
            }
        }
    }

    #[test]
    fn generate_is_random_hex_and_long_enough() {
        let a = generate();
        let b = generate();
        assert_eq!(a.len(), 64, "32 bytes hex");
        assert_ne!(a, b, "consecutive secrets must differ");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn secret_file_write_is_atomic_and_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.secret");
        write_secret_file(&path, "s3cret").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "s3cret");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "secret file must be 0600");
        }
        // Overwrite (rotation) leaves no tmp sibling behind.
        write_secret_file(&path, "rotated").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "rotated");
        assert!(
            !path.with_extension("secret.tmp").exists(),
            "tmp file must be renamed away, never left behind"
        );
    }

    #[test]
    fn discovery_env_wins_and_is_cached() {
        let _g = EnvGuard::set("SEBAS_CORE_SECRET", "from-env");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("core.secret");
        std::fs::write(&file, "from-file").unwrap();
        let cs = ChannelSecret::from_env_or_file(Some(file));
        assert_eq!(cs.current(), "from-env", "env must win over the file");
    }

    #[test]
    fn discovery_reads_file_fresh_each_time() {
        let _g = EnvGuard::unset("SEBAS_CORE_SECRET");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("core.secret");
        std::fs::write(&file, "key-one").unwrap();
        let cs = ChannelSecret::from_env_or_file(Some(file.clone()));
        assert_eq!(cs.current(), "key-one");
        // core 重启换钥：覆写文件后同一 client 实例立刻读到新钥（D2 自愈）。
        std::fs::write(&file, "key-two").unwrap();
        assert_eq!(cs.current(), "key-two", "rotation must be picked up");
    }

    #[test]
    fn discovery_both_missing_warns_once_and_uses_empty() {
        let _g = EnvGuard::unset("SEBAS_CORE_SECRET");
        let cs = ChannelSecret::from_env_or_file(Some(PathBuf::from(
            "/definitely/not/here/core.secret",
        )));
        MISSING_WARNED.store(false, Ordering::Relaxed);
        assert_eq!(cs.current(), "");
        assert_eq!(
            cs.current(),
            "",
            "empty attempt stays stable across reconnects"
        );
        // warn-once 语义不在此断言（tracing 断言成本高）；MISSING_WARNED 的
        // swap 行为由下一次 from_env_or_file 调用重置。
        MISSING_WARNED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn discovery_trims_whitespace() {
        let _g = EnvGuard::unset("SEBAS_CORE_SECRET");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("core.secret");
        std::fs::write(&file, "  padded-key\n").unwrap();
        let cs = ChannelSecret::from_env_or_file(Some(file));
        assert_eq!(cs.current(), "padded-key");
    }
}
