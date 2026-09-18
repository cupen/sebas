//! Session archive state for the WebUI.
//!
//! Archived sessions are moved out of the active session list into a separate
//! JSON file. Each archived session is read-only and may be restored to its
//! original project. Expired entries are cleaned up on startup and on every
//! list request.
//!
//! Persistence uses the same atomic tmp+rename pattern as `projects.rs`.
//!
//! polish-workbench-walkthrough-ux 1.1：归档文件路径收敛——解析顺序为
//! `SEBAS_ARCHIVE_PATH` → `<SEBAS_STATE_DB 目录>/archive.json` → 旧
//! `$HOME/.sebas/archive.json`（仅迁移源）。钉定 state 目录的部署（沙箱
//! 尤其）随之钉定归档文件；旧路径只读降级语义见 [`migrate_once`]。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// One archived session entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// 归档时刻的对话快照（polish-workbench-walkthrough-ux 2.1）：close 会
    /// 丢弃内存 transcript，归档视图要能只读回看——归档前在后端落一份。
    /// `#[serde(default)]` 兼容旧 archive.json（无字段 → 空对话）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transcript: Vec<sebas_dispatch::TurnEntry>,
}

/// The on-disk archive file format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct ArchiveFile {
    entries: Vec<ArchiveEntry>,
}

/// 迁移一次性护栏（进程级）：webui 生命周期内只尝试一次旧路径迁移。
static MIGRATED: AtomicBool = AtomicBool::new(false);
/// 迁移失败后的只读降级标记：resolved 位置放弃，继续读写旧路径。
static LEGACY_FALLBACK: AtomicBool = AtomicBool::new(false);
/// 成功迁移后待 /api/archive 转达前端的提示（take 即清）。
static MIGRATION_NOTICE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// 旧默认路径（legacy，仅迁移源/降级读写位）：`$SEBAS_HOME` 或
/// `$HOME/.sebas` 下的 `archive.json`。
fn legacy_default_path() -> PathBuf {
    let home = std::env::var("SEBAS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".sebas")
        });
    home.join("archive.json")
}

/// state DB 所在目录（`SEBAS_STATE_DB` 的父目录）。未设置/无父目录 → None。
fn state_db_dir() -> Option<PathBuf> {
    std::env::var("SEBAS_STATE_DB")
        .ok()
        .map(PathBuf::from)
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// resolved 归档路径（不含降级回退）：`SEBAS_ARCHIVE_PATH` >
/// `<SEBAS_STATE_DB 目录>/archive.json` > 旧默认路径。
fn resolved_archive_path() -> PathBuf {
    match std::env::var("SEBAS_ARCHIVE_PATH") {
        Ok(p) => PathBuf::from(p),
        Err(_) => match state_db_dir() {
            Some(dir) => dir.join("archive.json"),
            None => legacy_default_path(),
        },
    }
}

/// 当前生效的归档路径：迁移失败降级后 = 旧路径；否则 = resolved 路径。
fn archive_path() -> PathBuf {
    if LEGACY_FALLBACK.load(Ordering::Relaxed) {
        return legacy_default_path();
    }
    resolved_archive_path()
}

/// 迁移结果（单测可断言的纯判定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrationOutcome {
    /// 无事可做：resolved 已有文件 / legacy 不存在 / 两处同路径 / 显式 override。
    NotNeeded,
    /// legacy 文件已搬进 resolved 位置。
    Moved,
    /// 搬不动：调用方应降级为继续使用旧路径。
    Failed,
}

/// 把 legacy 归档文件一次性迁进 resolved 位置（polish-workbench-walkthrough-ux
/// 1.2）：resolved 无文件且 legacy 有 → rename；失败返回 `Failed`，调用方
/// warn 后继续用旧路径（只读降级，不空启覆盖）。
fn migrate(resolved: &Path, legacy: &Path) -> MigrationOutcome {
    if resolved == legacy || resolved.exists() || !legacy.exists() {
        return MigrationOutcome::NotNeeded;
    }
    let parent = match resolved.parent() {
        Some(p) => p,
        None => return MigrationOutcome::Failed,
    };
    match std::fs::create_dir_all(parent).and_then(|()| std::fs::rename(legacy, resolved)) {
        Ok(()) => MigrationOutcome::Moved,
        Err(_) => MigrationOutcome::Failed,
    }
}

/// 启动级一次性迁移（webui 首次触碰归档时执行）：成功记一条待转达提示；
/// 失败置降级标记并 warn——继续读旧路径，绝不空启。显式
/// `SEBAS_ARCHIVE_PATH` 时不动任何文件（override 语义最高优先）。
fn migrate_once() {
    if MIGRATED.swap(true, Ordering::Relaxed) {
        return;
    }
    if std::env::var("SEBAS_ARCHIVE_PATH").is_ok() || LEGACY_FALLBACK.load(Ordering::Relaxed) {
        return;
    }
    let resolved = resolved_archive_path();
    let legacy = legacy_default_path();
    match migrate(&resolved, &legacy) {
        MigrationOutcome::NotNeeded => {}
        MigrationOutcome::Moved => {
            tracing::info!(
                from = %legacy.display(),
                to = %resolved.display(),
                "migrated legacy archive.json to the state-db directory"
            );
            let message = format!(
                "归档记录已从 {} 迁移到 {}",
                legacy.display(),
                resolved.display()
            );
            *MIGRATION_NOTICE
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(message);
        }
        MigrationOutcome::Failed => {
            tracing::warn!(
                from = %legacy.display(),
                to = %resolved.display(),
                "failed to migrate legacy archive.json; continuing to read the legacy location"
            );
            LEGACY_FALLBACK.store(true, Ordering::Relaxed);
        }
    }
}

/// 取走待转达前端的迁移提示（`GET /api/archive` 响应携带；take 即清）。
pub fn take_migration_notice() -> Option<String> {
    MIGRATION_NOTICE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

fn load() -> Vec<ArchiveEntry> {
    migrate_once();
    let path = archive_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to read archive.json");
            return Vec::new();
        }
    };
    match serde_json::from_str::<ArchiveFile>(&raw) {
        Ok(file) => file.entries,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to parse archive.json, returning empty list");
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
/// configured retention period, snapshotting the conversation captured
/// before close. Returns the new entry.
pub fn archive_session(
    session_key: &str,
    project_path: &str,
    label: &str,
    retention_days: u64,
    transcript: Vec<sebas_dispatch::TurnEntry>,
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
        transcript,
    };
    entries.push(entry.clone());
    save(&entries)?;
    Ok(entry)
}

/// Fetch one archived entry (with its conversation snapshot) by session key.
pub fn entry(session_key: &str) -> Option<ArchiveEntry> {
    load().into_iter().find(|e| e.session_key == session_key)
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
        tracing::warn!(removed, error = %e, "failed to save after pruning expired archive entries");
    }
    removed
}

/// Check if a session key is archived (helper for the message gate).
pub fn is_archived(session_key: &str) -> bool {
    load().iter().any(|e| e.session_key == session_key)
}

/// 测试专用：archive env（SEBAS_ARCHIVE_PATH）的进程级串行锁句柄。
/// server.rs 的 workspace_root_tests 要对越界会话归档（会写 archive 文件），
/// 必须与本模块测试共用一把锁，否则并发测试会在彼此临界区内改写 env。
#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    tests::ARCHIVE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize archive tests because `SEBAS_ARCHIVE_PATH` is a process-global
    /// env var and Rust tests run in parallel.
    // pub(crate)：供上方 cfg(test) 的 test_env_lock 把锁交给同 crate 的路由级
    // 测试（workspace_root_tests），与 archive env 写放同一临界区。
    pub(crate) static ARCHIVE_TEST_LOCK: Mutex<()> = Mutex::new(());

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
        let entry = archive_session("sess_abc", "/home/user/proj", "My Session", 30, Vec::new()).unwrap();
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
        archive_session("sess_xyz", "/home/user/proj", "Test", 30, Vec::new()).unwrap();
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
        archive_session("sess_expired", "/home/user/proj", "Expired", 0, Vec::new()).unwrap();
        // Add one with a long retention.
        archive_session("sess_kept", "/home/user/proj", "Kept", 999, Vec::new()).unwrap();

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
        archive_session("sess_dup", "/home/user/proj", "First", 30, Vec::new()).unwrap();
        let result = archive_session("sess_dup", "/home/user/proj", "Second", 30, Vec::new());
        assert!(result.is_err(), "duplicate must be rejected");
        assert!(result.unwrap_err().contains("已归档"));
    }

    #[test]
    fn is_archived_checks_correctly() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        let _p = test_path("is_archived");
        assert!(!is_archived("sess_check"), "must not find unarchived key");
        archive_session("sess_check", "/home/user/proj", "Check", 30, Vec::new()).unwrap();
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

    // ── polish-workbench-walkthrough-ux 1.1/1.2：路径三级解析 + 迁移 ─────

    /// 沙箱式 env 写放（持锁调用）：设置/移除必须在同一把 ARCHIVE_TEST_LOCK
    /// 临界区内，防止并行测试互相改写进程全局 env。
    fn set_env(key: &str, value: Option<&str>) {
        // SAFETY: test-only, serialized by ARCHIVE_TEST_LOCK.
        unsafe {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }

    /// 1.1 三级解析：显式 `SEBAS_ARCHIVE_PATH` 最高优先。
    #[test]
    fn explicit_override_wins_resolution() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("sebas-archive-test/override_wins");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        set_env("SEBAS_ARCHIVE_PATH", Some(dir.join("override.json").to_str().unwrap()));
        set_env("SEBAS_STATE_DB", Some(dir.join("state").join("sebas.db").to_str().unwrap()));
        assert_eq!(resolved_archive_path(), dir.join("override.json"));
        set_env("SEBAS_ARCHIVE_PATH", None);
        set_env("SEBAS_STATE_DB", None);
    }

    /// 1.1 无 override 时归档跟随 `SEBAS_STATE_DB` 所在目录。
    #[test]
    fn state_db_dir_pins_the_archive() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("sebas-archive-test/state_db_pin");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        set_env("SEBAS_ARCHIVE_PATH", None);
        set_env("SEBAS_STATE_DB", Some(dir.join("sebas.db").to_str().unwrap()));
        assert_eq!(resolved_archive_path(), dir.join("archive.json"));
        set_env("SEBAS_STATE_DB", None);
    }

    /// 1.1 无 `SEBAS_STATE_DB` 时回退旧默认路径（legacy 形态）。
    #[test]
    fn no_state_db_falls_back_to_legacy_default() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        set_env("SEBAS_ARCHIVE_PATH", None);
        set_env("SEBAS_STATE_DB", None);
        set_env("SEBAS_HOME", Some("/tmp/sebas-archive-test/home"));
        assert_eq!(resolved_archive_path(), PathBuf::from("/tmp/sebas-archive-test/home/archive.json"));
        set_env("SEBAS_HOME", None);
    }

    /// 1.2 迁移成功：resolved 无文件且 legacy 有 → rename 到新位置。
    #[test]
    fn migration_moves_legacy_forward() {
        let dir = std::env::temp_dir().join("sebas-archive-test/migrate_ok");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("legacy")).unwrap();
        let legacy = dir.join("legacy/archive.json");
        std::fs::write(&legacy, r#"{"entries":[]}"#).unwrap();
        let resolved = dir.join("resolved/archive.json");
        assert_eq!(migrate(&resolved, &legacy), MigrationOutcome::Moved);
        assert!(resolved.exists(), "resolved file must exist after migration");
        assert!(!legacy.exists(), "legacy file must be gone after rename");
    }

    /// 1.2 迁移失败（父目录不可创建）：返回 Failed，调用方降级读旧路径。
    #[test]
    fn migration_failure_reports_failed() {
        let dir = std::env::temp_dir().join("sebas-archive-test/migrate_fail");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("legacy")).unwrap();
        let legacy = dir.join("legacy/archive.json");
        std::fs::write(&legacy, r#"{"entries":[]}"#).unwrap();
        // resolved 的父路径落在一段文件上 → create_dir_all 必败。
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "not a dir").unwrap();
        let resolved = blocker.join("archive.json");
        assert_eq!(migrate(&resolved, &legacy), MigrationOutcome::Failed);
        assert!(legacy.exists(), "legacy file must survive a failed move");
    }

    /// 1.2 不需要迁移的形态：resolved 已有文件 / legacy 不存在 / 同路径。
    #[test]
    fn migration_skipped_when_not_needed() {
        let dir = std::env::temp_dir().join("sebas-archive-test/migrate_skip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let resolved = dir.join("archive.json");
        let legacy = dir.join("legacy.json");
        // resolved 已存在。
        std::fs::write(&resolved, r#"{"entries":[]}"#).unwrap();
        std::fs::write(&legacy, r#"{"entries":[]}"#).unwrap();
        assert_eq!(migrate(&resolved, &legacy), MigrationOutcome::NotNeeded);
        // legacy 不存在。
        std::fs::remove_file(&resolved).unwrap();
        std::fs::remove_file(&legacy).unwrap();
        assert_eq!(migrate(&resolved, &legacy), MigrationOutcome::NotNeeded);
        // 同路径。
        assert_eq!(migrate(&resolved, &resolved), MigrationOutcome::NotNeeded);
    }

    /// 2.1：归档条目携带对话快照，读回逐一对应。
    #[test]
    fn archived_entry_round_trips_transcript() {
        let _lock = ARCHIVE_TEST_LOCK.lock().unwrap();
        let _p = test_path("transcript_round_trip");
        let transcript = vec![
            sebas_dispatch::TurnEntry::prompt(0, "hello"),
            sebas_dispatch::TurnEntry::markdown(1, "world"),
        ];
        archive_session("sess_t", "/proj", "T", 30, transcript.clone()).unwrap();
        let entry = entry("sess_t").expect("entry must be found");
        assert_eq!(entry.transcript, transcript);
    }
}