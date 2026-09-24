//! Core session channel secret: **core 侧**的生成与原子落盘
//! （harden-core-channel-deployment, design D1/D2/D5）。
//!
//! **解析（发现）已下沉 `sebas_ipc::secret`**（unify-ipc-protocol-home 4.2）：
//! 每一个通道客户端（standalone webui、im、router 的状态订阅）此前各自
//! 复刻一份「env → secret 文件」的解析；现在只有一份共享实现，这里原位
//! `pub use` 保持既有公开路径不变。
//!
//! 本模块只剩**文件生命周期**：core 在 arm 时把 secret 原子写入
//! （tmp + rename，unix 上 0600），且**有意**不在优雅退出时删除它——socket
//! 文件才是「core 已死」的权威信号，残留的 secret 文件无害（没有活 socket
//! 就到不了握手）。

use std::path::Path;

pub use sebas_ipc::secret::{ChannelSecret, ChannelSecret as SecretSource, SECRET_FILE_NAME};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
