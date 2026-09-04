//! Session archive state for the WebUI.
//!
//! Archived sessions are moved out of the active session list into a separate
//! JSON file (`~/.sebas/archive.json`). Each archived session is read-only
//! and may be restored to its original project. Expired entries are cleaned
//! up on startup and on every list request.
//!
//! Persistence uses the same atomic tmp+rename pattern as `projects.rs`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// One archived session entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// The encoded session key.
    pub session_key: String,
    /// The project path the session belonged to when archived.
    pub project_path: String,
    /// Human-readable session label.
    pub label: String,
    /// Unix seconds when the session was archived.
    pub archived_at: u64,
    /// Unix seconds after which this entry may be permanently deleted.
    pub retention_deadline: u64,
}

/// The on-disk archive file format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ArchiveFile {
    entries: Vec<ArchiveEntry>,
}

fn default_path() -> PathBuf {
    let home = std::env::var("SEBAS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".sebas")
        });
    home.join("archive.json")
}

fn archive_path() -> PathBuf {
    std::env::var("SEBAS_ARCHIVE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_path())
}

fn load() -> Vec<ArchiveEntry> {
    let path = archive_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "archive.json 读取失败");
            return Vec::new();
        }
    };
    match serde_json::from_str::<ArchiveFile>(&raw) {
        Ok(file) => file.entries,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "archive.json 解析失败，返回空列表");
            Vec::new()
        }
    }
}

fn save(entries: &[ArchiveEntry]) -> Result<(), String> {
    let path = archive_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
    }
    let file = ArchiveFile {
        entries: entries.to_vec(),
    };
    let body = serde_json::to_string_pretty(&file)
        .map_err(|e| format!("序列化 archive.json 失败: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &body)
        .map_err(|e| format!("写入临时文件 {} 失败: {e}", tmp.display()))?;
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&tmp) {
        file.sync_all().ok();
    }
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("重命名 {} → {} 失败: {e}", tmp.display(), path.display()))?;
    if let Some(parent) = path.parent()
        && let Ok(dir) = std::fs::OpenOptions::new().read(true).open(parent)
    {
        dir.sync_all().ok();
    }
    Ok(())
}

/// Return the current unix timestamp in seconds.
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// List all archived sessions.
pub fn list() -> Vec<ArchiveEntry> {
    load()
}

/// Archive a session: create an entry with the given metadata and the
/// configured retention period. Returns the new entry.
pub fn archive_session(
    session_key: &str,
    project_path: &str,
    label: &str,
    retention_days: u64,
) -> Result<ArchiveEntry, String> {
    let mut entries = load();
    // Reject duplicates.
    if entries.iter().any(|e| e.session_key == session_key) {
        return Err(format!("会话已归档: {session_key}"));
    }
    let now = now_unix();
    let retention_secs = retention_days * 86400;
    let entry = ArchiveEntry {
        session_key: session_key.to_string(),
        project_path: project_path.to_string(),
        label: label.to_string(),
        archived_at: now,
        retention_deadline: now + retention_secs,
    };
    entries.push(entry.clone());
    save(&entries)?;
    Ok(entry)
}

/// Restore an archived session: remove it from the archive and return its
/// data. Returns `None` if the session key was not found.
pub fn restore_session(session_key: &str) -> Option<ArchiveEntry> {
    let mut entries = load();
    let idx = entries.iter().position(|e| e.session_key == session_key)?;
    let entry = entries.remove(idx);
    save(&entries).ok()?;
    Some(entry)
}

/// Remove expired entries from the archive. Returns the number of removed
/// entries.
pub fn cleanup_expired() -> usize {
    let now = now_unix();
    let mut entries = load();
    let before = entries.len();
    entries.retain(|e| e.retention_deadline > now);
    let removed = before - entries.len();
    if removed > 0
        && let Err(e) = save(&entries)
    {
        tracing::warn!(removed, error = %e, "清理过期归档条目后保存失败");
    }
    removed
}

/// Check if a session key is archived (helper for the message gate).
pub fn is_archived(session_key: &str) -> bool {
    load().iter().any(|e| e.session_key == session_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize archive tests because `SEBAS_ARCHIVE_PATH` is a process-global
    /// env var and Rust tests run in parallel.
    static ARCHIVE_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Use a unique directory per test inside a shared temp root.
    fn test_path(test_name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("sebas-archive-test")
            .join(test_name);
        // Clean up from previous runs.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: test-only, serialized by ARCHIVE_TEST_LOCK.
        unsafe { std::env::set_var("SEBAS_ARCHIVE_PATH", dir.join("archive.json")); }
        dir.join("archive.json")
    }

    #[test]
    fn empty_file_returns_empty_list() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        let p = test_path("empty_file");
        // Ensure the file does not exist.
        let _ = std::fs::remove_file(&p);
        let entries = list();
        assert!(entries.is_empty(), "no file should return empty list");
    }

    #[test]
    fn add_and_list_entry() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        let _p = test_path("add_and_list");
        let entry = archive_session("sess_abc", "/home/user/proj", "My Session", 30).unwrap();
        assert_eq!(entry.session_key, "sess_abc");
        assert_eq!(entry.project_path, "/home/user/proj");
        assert_eq!(entry.label, "My Session");
        assert!(entry.archived_at > 0);
        let deadline = entry.archived_at + 30 * 86400;
        assert_eq!(entry.retention_deadline, deadline);

        let entries = list();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].session_key, "sess_abc");
    }

    #[test]
    fn remove_entry() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        let _p = test_path("remove_entry");
        archive_session("sess_xyz", "/home/user/proj", "Test", 30).unwrap();
        let restored = restore_session("sess_xyz");
        assert!(restored.is_some(), "must find the entry");
        assert_eq!(restored.unwrap().session_key, "sess_xyz");

        // After restore, the entry should be gone.
        let entries = list();
        assert!(entries.is_empty(), "entry must be removed after restore");
    }

    #[test]
    fn restore_nonexistent_returns_none() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        let _p = test_path("restore_nonexistent");
        let result = restore_session("sess_nonexistent");
        assert!(result.is_none(), "must return None for unknown key");
    }

    #[test]
    fn cleanup_expired_removes_old_entries() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        let _p = test_path("cleanup_expired");
        // Add an entry with 0-day retention — it should be expired immediately.
        archive_session("sess_expired", "/home/user/proj", "Expired", 0).unwrap();
        // Add one with a long retention.
        archive_session("sess_kept", "/home/user/proj", "Kept", 999).unwrap();

        let removed = cleanup_expired();
        assert_eq!(removed, 1, "the expired entry should be removed");

        let entries = list();
        assert_eq!(entries.len(), 1, "only the kept entry remains");
        assert_eq!(entries[0].session_key, "sess_kept");
    }

    #[test]
    fn duplicate_archive_rejected() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        let _p = test_path("dup_rejected");
        archive_session("sess_dup", "/home/user/proj", "First", 30).unwrap();
        let result = archive_session("sess_dup", "/home/user/proj", "Second", 30);
        assert!(result.is_err(), "duplicate must be rejected");
        assert!(result.unwrap_err().contains("已归档"));
    }

    #[test]
    fn is_archived_checks_correctly() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        let _p = test_path("is_archived");
        assert!(!is_archived("sess_check"), "must not find unarchived key");
        archive_session("sess_check", "/home/user/proj", "Check", 30).unwrap();
        assert!(is_archived("sess_check"), "must find archived key");
    }

    #[test]
    fn unparseable_file_returns_empty() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        let p = test_path("unparseable");
        // Write garbage to the file.
        std::fs::write(&p, "not json").unwrap();
        let entries = list();
        assert!(entries.is_empty(), "unparseable file should return empty list");
    }
}