//! Core session channel secret file (harden-core-channel-deployment D1/D2/D5).
//!
//! The core always arms its session channel (no opt-out): the handshake
//! secret comes from `SEBAS_CORE_SECRET` when the watchdog injected one,
//! otherwise the core mints a random secret at startup and publishes it via
//! the secret file so independently-started clients (standalone webui, im,
//! router) can discover it. Resolution order everywhere (core + clients):
//! env → secret file → empty (clients warn once and keep trying).
//!
//! - Default path: `<config file dir>/core.secret`, overrideable with
//!   `[watchdog.core] secret_file` (explicit TOML wins) or the
//!   `SEBAS_CORE_SECRET_FILE` env pinned by the watchdog for children whose
//!   own `-c` derivation is unavailable (the router crate).
//! - Writes are atomic (tmp + rename); unix mode 0600. Graceful exit keeps
//!   the file: a stale secret is harmless because clients never get past the
//!   handshake while the socket is absent (D5).

use std::path::{Path, PathBuf};

/// Env carrying the watchdog-injected handshake secret (env-first, unchanged).
pub const SECRET_ENV: &str = "SEBAS_CORE_SECRET";
/// Env pinning the secret file path (set by the watchdog for supervised
/// children; honored when the TOML key is absent).
pub const SECRET_FILE_ENV: &str = "SEBAS_CORE_SECRET_FILE";
/// Default file name next to the config file (D1).
pub const SECRET_FILE_NAME: &str = "core.secret";

/// Resolve the secret file path: explicit `[watchdog.core] secret_file`
/// wins, then `SEBAS_CORE_SECRET_FILE`, then `<config dir>/core.secret`.
/// A config path without a parent dir (bare file name) falls back to the
/// process working directory.
pub fn secret_file_path(explicit: Option<&str>, config_path: &Path) -> PathBuf {
    if let Some(p) = explicit
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var(SECRET_FILE_ENV)
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    let dir = config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    dir.join(SECRET_FILE_NAME)
}

/// Mint a fresh handshake secret: 32 bytes of OS entropy, hex-encoded.
/// Falls back to a hashed pid+time+counter mix only when OS entropy is
/// unavailable (e.g. `/dev/urandom` unreadable) — still unique per boot,
/// and the real boundary stays uid + 0600 (D1 risk note).
pub fn generate_secret() -> String {
    let mut buf = [0u8; 32];
    if read_os_entropy(&mut buf) {
        return hex_encode(&buf);
    }
    // Fallback: unique-per-boot mix (never empty, never constant).
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut h = DefaultHasher::new();
    std::process::id().hash(&mut h);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut h);
    COUNTER
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .hash(&mut h);
    std::thread::current().id().hash(&mut h);
    format!("fb-{:016x}{:016x}", h.finish(), {
        let mut h2 = DefaultHasher::new();
        buf.hash(&mut h2);
        h2.finish()
    })
}

#[cfg(unix)]
fn read_os_entropy(buf: &mut [u8]) -> bool {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf).map(|_| ()))
        .is_ok()
        && buf.iter().any(|b| *b != 0)
}

#[cfg(not(unix))]
fn read_os_entropy(_buf: &mut [u8]) -> bool {
    false
}

fn hex_encode(buf: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(buf.len() * 2);
    for b in buf {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// Atomically publish the secret (tmp + rename). Creates the parent dir;
/// unix pins the file to 0600 (before and after the rename, so neither the
/// tmp nor the final path is ever group-readable).
pub fn write_secret_file(path: &Path, secret: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("secret.tmp");
    std::fs::write(&tmp, secret.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Read a published secret: trimmed, empty/missing → None.
pub fn read_secret_file(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let secret = raw.trim().to_string();
    (!secret.is_empty()).then_some(secret)
}

/// Resolve the handshake secret: `SEBAS_CORE_SECRET` env first (non-empty),
/// else the secret file, else empty string (callers warn, never crash).
pub fn resolve_secret(secret_file: &Path) -> String {
    if let Ok(v) = std::env::var(SECRET_ENV)
        && !v.is_empty()
    {
        return v;
    }
    read_secret_file(secret_file).unwrap_or_default()
}

/// Whether the env currently pins the secret (clients cache it, zero file
/// reads on the hot path; file mode re-reads before every connection).
pub fn env_pins_secret() -> bool {
    std::env::var(SECRET_ENV)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Process-wide serializer for tests that mutate `SEBAS_CORE_SECRET` /
/// `SEBAS_CORE_SECRET_FILE`: env is process-global, so every test that
/// touches it (here, `run::core_secret_tests`, client discovery tests)
/// must hold this lock.
#[cfg(test)]
pub(crate) static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_key_beats_default() {
        let p = secret_file_path(Some("/etc/sebas/x.secret"), Path::new("/cfg/config.toml"));
        assert_eq!(p, PathBuf::from("/etc/sebas/x.secret"));
    }

    #[test]
    fn default_derives_from_config_dir() {
        let _env = ENV_TEST_LOCK.lock().unwrap();
        // SAFETY: ENV_TEST_LOCK 已持有，无并发 env 访问。
        unsafe { std::env::remove_var(SECRET_FILE_ENV) };
        let p = secret_file_path(None, Path::new("/cfg/sub/config.toml"));
        assert_eq!(p, PathBuf::from("/cfg/sub/core.secret"));
        let p = secret_file_path(Some(""), Path::new("/cfg/config.toml"));
        assert_eq!(p, PathBuf::from("/cfg/core.secret"));
    }

    #[test]
    fn bare_config_name_falls_back_to_cwd() {
        let _env = ENV_TEST_LOCK.lock().unwrap();
        // SAFETY: ENV_TEST_LOCK 已持有。
        unsafe { std::env::remove_var(SECRET_FILE_ENV) };
        let p = secret_file_path(None, Path::new("config.toml"));
        assert_eq!(p, PathBuf::from("./core.secret"));
    }

    #[test]
    fn generated_secrets_are_unique_and_hex() {
        let a = generate_secret();
        let b = generate_secret();
        assert!(!a.is_empty() && !b.is_empty());
        assert_ne!(a, b, "two mints must differ");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    #[cfg(unix)]
    fn write_is_atomic_with_mode_0600_and_read_roundtrips() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.secret");
        write_secret_file(&path, "s3cr3t-value").unwrap();
        assert!(!dir.path().join("core.secret.tmp").exists());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(read_secret_file(&path).as_deref(), Some("s3cr3t-value"));
        // Overwrite replaces atomically with the new value.
        write_secret_file(&path, "rotated").unwrap();
        assert_eq!(read_secret_file(&path).as_deref(), Some("rotated"));
    }

    #[test]
    fn resolve_prefers_env_over_file() {
        let _env = ENV_TEST_LOCK.lock().unwrap();
        // SAFETY: ENV_TEST_LOCK 已持有；结束前恢复。
        unsafe {
            std::env::set_var(SECRET_ENV, "from-env");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.secret");
        write_secret_file(&path, "from-file").unwrap();
        assert_eq!(resolve_secret(&path), "from-env");
        unsafe {
            std::env::remove_var(SECRET_ENV);
        }
        assert_eq!(resolve_secret(&path), "from-file");
        assert_eq!(resolve_secret(&dir.path().join("missing.secret")), "");
        unsafe {
            std::env::remove_var(SECRET_ENV);
        }
    }
}
