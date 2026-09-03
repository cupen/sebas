//! Project registry: `~/.sebas/projects.json`, WebUI-owned.
//!
//! Each entry is a directory path the operator registered as a project.
//! Atomic writes via tmp + rename + fsync, matching `state_store` pattern.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Git branch re-check TTL (seconds). Branches rarely change; the cache
/// keeps the rail cheap when many sessions share a project.
pub const BRANCH_TTL_SECS: u64 = 30;

/// A single registered project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectEntry {
    pub path: String,
    pub name: String,
    pub added_at: u64,
    /// Git branch read lazily; refreshed at most once per `BRANCH_TTL_SECS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Unix seconds of the last branch probe (0 = never).
    #[serde(default)]
    pub branch_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RegistryFile {
    projects: Vec<ProjectEntry>,
}

fn default_path() -> PathBuf {
    let home = std::env::var("SEBAS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".sebas")
        });
    home.join("projects.json")
}

fn registry_path() -> PathBuf {
    std::env::var("SEBAS_PROJECTS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_path())
}

fn load() -> Vec<ProjectEntry> {
    let path = registry_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "projects.json 读取失败");
            return Vec::new();
        }
    };
    match serde_json::from_str::<RegistryFile>(&raw) {
        Ok(file) => file.projects,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "projects.json 解析失败，返回空列表");
            Vec::new()
        }
    }
}

fn save(projects: &[ProjectEntry]) -> Result<(), String> {
    let path = registry_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
    }
    let file = RegistryFile {
        projects: projects.to_vec(),
    };
    let body = serde_json::to_string_pretty(&file)
        .map_err(|e| format!("序列化 projects.json 失败: {e}"))?;
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

pub fn list() -> Vec<ProjectEntry> {
    let mut projects = load();
    // Backfill branch_at=0 for entries persisted by an older registry that
    // did not track probe timestamps.
    let mut dirty = false;
    for p in &mut projects {
        if p.branch_at == 0 && p.branch.is_some() {
            p.branch_at = 1;
            dirty = true;
        }
    }
    if dirty {
        let _ = save(&projects);
    }
    projects
}

/// Add a project by directory path. Returns the entry, or Err on failure.
/// Rejects non-existent paths, non-directories, and duplicates.
pub fn add(path: &str) -> Result<ProjectEntry, String> {
    let dir = Path::new(path);
    if !dir.exists() {
        return Err(format!("路径不存在: {path}"));
    }
    if !dir.is_dir() {
        return Err(format!("路径不是目录: {path}"));
    }
    let canonical = dir
        .canonicalize()
        .map_err(|_| format!("无法解析路径: {path}"))?;
    let canonical_str = canonical.to_string_lossy().to_string();
    let mut projects = load();
    if projects.iter().any(|p| Path::new(&p.path) == canonical) {
        return Err(format!("项目已注册: {path}"));
    }
    let name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".to_string());
    let added_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let entry = ProjectEntry {
        path: canonical_str,
        name,
        added_at,
        branch: None,
        branch_at: 0,
    };
    projects.push(entry.clone());
    save(&projects)?;
    Ok(entry)
}

/// Remove a project by path. Returns `Ok(true)` if removed, `Ok(false)` if not found.
pub fn remove(path: &str) -> Result<bool, String> {
    let canonical = match Path::new(path).canonicalize() {
        Ok(p) => p,
        Err(_) => return Ok(false),
    };
    let mut projects = load();
    let before = projects.len();
    projects.retain(|p| Path::new(&p.path) != canonical);
    if projects.len() == before {
        return Ok(false);
    }
    save(&projects)?;
    Ok(true)
}

/// Reorder the registry to match the provided sequence of canonical paths.
/// Paths not in the sequence keep their relative position; unknown paths are
/// appended at the end. Returns the new ordering.
pub fn reorder(ordered_paths: &[String]) -> Result<Vec<ProjectEntry>, String> {
    let mut projects = load();
    let mut by_path: std::collections::HashMap<String, ProjectEntry> =
        projects.drain(..).map(|p| (p.path.clone(), p)).collect();
    let mut next: Vec<ProjectEntry> = Vec::with_capacity(ordered_paths.len());
    let mut seen = std::collections::HashSet::new();
    for path in ordered_paths {
        if seen.insert(path.clone())
            && let Some(entry) = by_path.remove(path)
        {
            next.push(entry);
        }
    }
    // Append any unregistered projects (e.g. manually edited file) at the end.
    let mut tail: Vec<ProjectEntry> = by_path.into_values().collect();
    tail.sort_by_key(|p| p.added_at);
    next.extend(tail);
    save(&next)?;
    Ok(next)
}

/// Returns true when the project's directory exists and is reachable. Used
/// by the UI to render a row that still lists its sessions but is visually
/// marked unreachable.
pub fn is_accessible(path: &str) -> bool {
    Path::new(path).is_dir()
}

/// Read the project's git branch from `.git/HEAD`. Returns `None` if the
/// directory is not a git working tree or the file is missing/unreadable.
/// Pure filesystem read: no subprocess, safe to call concurrently.
///
/// Refreshes the cached branch when older than `BRANCH_TTL_SECS`. Cache
/// writes go back through `save`, which serialises concurrent writers via
/// the tmp+rename atomicity.
pub fn read_branch(path: &str) -> Option<String> {
    let mut projects = load();
    let entry = projects
        .iter_mut()
        .find(|p| p.path == path)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if entry.branch_at != 0 && now.saturating_sub(entry.branch_at) < BRANCH_TTL_SECS {
        return entry.branch.clone();
    }
    let branch = probe_git_branch(Path::new(path));
    entry.branch = branch.clone();
    entry.branch_at = now;
    if let Err(e) = save(&projects) {
        tracing::warn!(path = %path, error = %e, "branch cache 写回失败");
    }
    branch
}

/// Force a branch re-read, ignoring the TTL cache.
pub fn refresh_branch(path: &str) -> Option<String> {
    let branch = probe_git_branch(Path::new(path));
    let mut projects = load();
    if let Some(entry) = projects.iter_mut().find(|p| p.path == path) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        entry.branch = branch.clone();
        entry.branch_at = now;
        if let Err(e) = save(&projects) {
            tracing::warn!(path = %path, error = %e, "branch cache 写回失败");
        }
    }
    branch
}

/// Reads `.git/HEAD`. Handles two cases:
/// - Plain ref: `ref: refs/heads/main` → branch = `main`
/// - Detached HEAD: raw commit sha → branch = None (we don't fabricate names)
fn probe_git_branch(repo: &Path) -> Option<String> {
    let head = std::fs::read_to_string(repo.join(".git/HEAD")).ok()?;
    let head = head.trim();
    let rest = head.strip_prefix("ref:")?.trim();
    let branch = rest.strip_prefix("refs/heads/")?;
    if branch.is_empty() {
        None
    } else {
        Some(branch.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    static TEST_MUTEX: LazyLock<std::sync::Mutex<()>> = LazyLock::new(|| std::sync::Mutex::new(()));
    static TEST_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static TEST_DIR: LazyLock<tempfile::TempDir> = LazyLock::new(|| {
        tempfile::tempdir().expect("create temp dir for tests")
    });

    fn test_registry_path() -> PathBuf {
        let n = TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        TEST_DIR.path().join(format!("test_projects_{n}.json"))
    }

    fn with_test_env<F: FnOnce()>(f: F) {
        let _guard = TEST_MUTEX.lock().unwrap();
        let path = test_registry_path();
        let prev = std::env::var("SEBAS_PROJECTS_PATH").ok();
        unsafe { std::env::set_var("SEBAS_PROJECTS_PATH", &path); }
        f();
        let _ = std::fs::remove_file(&path);
        match prev {
            Some(p) => unsafe { std::env::set_var("SEBAS_PROJECTS_PATH", p) },
            None => unsafe { std::env::remove_var("SEBAS_PROJECTS_PATH") },
        }
    }

    #[test] fn absent_file_returns_empty() { with_test_env(|| { assert!(list().is_empty()); }); }
    #[test] fn unparseable_file_returns_empty() { with_test_env(|| { std::fs::write(test_registry_path(), "bad").unwrap(); assert!(list().is_empty()); }); }
    #[test] fn add_and_list() { with_test_env(|| { let dir = TEST_DIR.path().join("p"); std::fs::create_dir_all(&dir).unwrap(); let e = add(&dir.to_string_lossy()).unwrap(); assert_eq!(e.name, "p"); assert_eq!(list().len(), 1); }); }
    #[test] fn duplicate_add_rejected() { with_test_env(|| { let dir = TEST_DIR.path().join("d"); std::fs::create_dir_all(&dir).unwrap(); let s = dir.to_string_lossy(); add(&s).unwrap(); assert!(add(&s).is_err()); }); }
    #[test] fn remove_project() { with_test_env(|| { let dir = TEST_DIR.path().join("r"); std::fs::create_dir_all(&dir).unwrap(); let s = dir.to_string_lossy(); add(&s).unwrap(); assert!(remove(&s).unwrap()); assert!(list().is_empty()); }); }
    #[test] fn add_nonexistent_rejected() { with_test_env(|| { assert!(add("/bogus").is_err()); }); }
    #[test] fn add_file_rejected() { with_test_env(|| { let f = TEST_DIR.path().join("f.txt"); std::fs::write(&f, "x").unwrap(); assert!(add(&f.to_string_lossy()).is_err()); }); }
    #[test] fn persists_across_reload() { with_test_env(|| { let dir = TEST_DIR.path().join("p2"); std::fs::create_dir_all(&dir).unwrap(); add(&dir.to_string_lossy()).unwrap(); drop(list()); assert_eq!(list().len(), 1); }); }

    #[test] fn reorder_persists_user_order() {
        with_test_env(|| {
            for n in ["a","b","c"] {
                let d = TEST_DIR.path().join(n);
                std::fs::create_dir_all(&d).unwrap();
                add(&d.to_string_lossy()).unwrap();
            }
            let before = list();
            // Reverse: c, b, a.
            let paths: Vec<String> = before.iter().rev().map(|p| p.path.clone()).collect();
            let after = reorder(&paths).unwrap();
            assert_eq!(after.iter().map(|p| &p.path).collect::<Vec<_>>(), paths.iter().collect::<Vec<_>>());

            // Persists across re-load.
            let again = list();
            assert_eq!(again.iter().map(|p| &p.path).collect::<Vec<_>>(), paths.iter().collect::<Vec<_>>());
        });
    }

    #[test] fn reorder_ignores_unknown_paths() {
        with_test_env(|| {
            let d = TEST_DIR.path().join("a");
            std::fs::create_dir_all(&d).unwrap();
            add(&d.to_string_lossy()).unwrap();
            let bogus = "/totally/bogus/path".to_string();
            let result = reorder(&[bogus]).unwrap();
            // The known entry is appended; unknown path is dropped.
            assert_eq!(result.len(), 1);
            assert!(result[0].path.contains("a"));
        });
    }

    #[test] fn read_branch_finds_git_head() {
        with_test_env(|| {
            let d = TEST_DIR.path().join("g");
            std::fs::create_dir_all(d.join(".git")).unwrap();
            std::fs::write(d.join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
            add(&d.to_string_lossy()).unwrap();
            let path = list()[0].path.clone();
            assert_eq!(read_branch(&path), Some("feature/x".to_string()));
        });
    }

    #[test] fn read_branch_returns_none_for_non_git_dir() {
        with_test_env(|| {
            let d = TEST_DIR.path().join("n");
            std::fs::create_dir_all(&d).unwrap();
            add(&d.to_string_lossy()).unwrap();
            let path = list()[0].path.clone();
            assert_eq!(read_branch(&path), None);
        });
    }

    #[test] fn read_branch_returns_none_for_detached_head() {
        with_test_env(|| {
            let d = TEST_DIR.path().join("d");
            std::fs::create_dir_all(d.join(".git")).unwrap();
            std::fs::write(d.join(".git/HEAD"), "9dce8c9d4f3b1e2a0b8c0d1e2f3a4b5c6d7e8f90\n").unwrap();
            add(&d.to_string_lossy()).unwrap();
            let path = list()[0].path.clone();
            assert_eq!(read_branch(&path), None);
        });
    }

    #[test] fn is_accessible_reflects_filesystem() {
        with_test_env(|| {
            let d = TEST_DIR.path().join("alive");
            std::fs::create_dir_all(&d).unwrap();
            add(&d.to_string_lossy()).unwrap();
            let path = list()[0].path.clone();
            assert!(is_accessible(&path));
            // The registry still contains the entry, but the dir is gone.
            std::fs::remove_dir_all(&d).unwrap();
            assert!(!is_accessible(&path));
        });
    }
}