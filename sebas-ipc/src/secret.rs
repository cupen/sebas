//! 通道 secret 的**共享发现实现**（harden-core-channel-deployment D1/D2）。
//!
//! 每一个 core session channel 客户端（standalone webui、im、router 的状态
//! 订阅）都用同一份解析：`SEBAS_CORE_SECRET` env 优先（构造期缓存，零每次
//! 连接成本）；否则**每次连接尝试前**重读 secret 文件——core 重启换钥后
//! 客户端靠自己的重连退避天然自愈，不需要通知通道。两者皆缺省时 warn 一次
//! 并以空 secret 尝试握手（诚实，不静默；服务端会关闭连接）。
//!
//! 这里放的是**解析**，不是文件生命周期：secret 文件的生成与原子落盘属于
//! core 进程（根 crate 的 `core_channel::secret`），不进本 crate——它需要
//! CSPRNG 与文件权限，与「协议之家」的准入面无关。
//!
//! 本模块由 `unify-ipc-protocol-home` 4.2 从根 crate 搬入：此前 router 自己
//! 复刻了一份同样的解析（`SEBAS_ROUTER_CONFIG` 目录下的 `core.secret`），
//! 两份实现靠注释维持同步。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Default secret file name, resolved next to the config file (D1).
pub const SECRET_FILE_NAME: &str = "core.secret";

/// Warn-once flag shared by every discovery in this process: missing secret
/// is a startup-visible condition, not a per-reconnect log flood.
static MISSING_WARNED: AtomicBool = AtomicBool::new(false);

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

/// 供测试重置 warn-once 状态（生产路径不调用）。
#[doc(hidden)]
pub fn reset_missing_warn() {
    MISSING_WARNED.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同进程并行测试都在读写 `SEBAS_CORE_SECRET`：guard 持锁到 drop，串行化。
    struct EnvGuard {
        name: &'static str,
        prev_present: bool,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    impl EnvGuard {
        fn set(name: &'static str, value: &str) -> Self {
            let _lock = lock();
            let prev_present = std::env::var(name).is_ok();
            unsafe { std::env::set_var(name, value) };
            Self {
                name,
                prev_present,
                _lock,
            }
        }
        fn unset(name: &'static str) -> Self {
            let _lock = lock();
            let prev_present = std::env::var(name).is_ok();
            unsafe { std::env::remove_var(name) };
            Self {
                name,
                prev_present,
                _lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if !self.prev_present {
                unsafe { std::env::remove_var(self.name) };
            }
        }
    }

    #[test]
    fn discovery_env_wins_and_is_cached() {
        let _g = EnvGuard::set("SEBAS_CORE_SECRET", "from-env");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(SECRET_FILE_NAME);
        std::fs::write(&file, "from-file").unwrap();
        let cs = ChannelSecret::from_env_or_file(Some(file));
        assert_eq!(cs.current(), "from-env", "env must win over the file");
    }

    #[test]
    fn discovery_reads_file_fresh_each_time() {
        let _g = EnvGuard::unset("SEBAS_CORE_SECRET");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(SECRET_FILE_NAME);
        std::fs::write(&file, "key-one").unwrap();
        let cs = ChannelSecret::from_env_or_file(Some(file.clone()));
        assert_eq!(cs.current(), "key-one");
        // core 重启换钥：覆写文件后同一 client 实例立刻读到新钥（D2 自愈）。
        std::fs::write(&file, "key-two").unwrap();
        assert_eq!(cs.current(), "key-two", "rotation must be picked up");
    }

    #[test]
    fn discovery_both_missing_uses_empty_attempt() {
        let _g = EnvGuard::unset("SEBAS_CORE_SECRET");
        reset_missing_warn();
        let cs = ChannelSecret::from_env_or_file(Some(PathBuf::from(
            "/definitely/not/here/core.secret",
        )));
        assert_eq!(cs.current(), "");
        assert_eq!(cs.current(), "", "empty attempt stays stable across reconnects");
        reset_missing_warn();
    }

    #[test]
    fn discovery_trims_whitespace() {
        let _g = EnvGuard::unset("SEBAS_CORE_SECRET");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(SECRET_FILE_NAME);
        std::fs::write(&file, "  padded-key\n").unwrap();
        let cs = ChannelSecret::from_env_or_file(Some(file));
        assert_eq!(cs.current(), "padded-key");
    }
}