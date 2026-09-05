//! Directory browser for the Add Project dialog's path picker.
//!
//! Exposes `GET /api/fs/browse-dirs?path=<path>&root=<root>` which returns the
//! immediate child directories of a path, scoped to a root directory for
//! safety. The default root is `/` — the full filesystem is browsable, and
//! the client (folder-picker component) navigates from there.
//!
//! The `root` query parameter sets the scope: all paths are resolved relative
//! to and bounded within it. When omitted, root defaults to `/`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One entry in a directory listing.
#[derive(Debug, Clone, Serialize)]
pub struct FsEntry {
    pub name: String,
    pub is_dir: bool,
    /// Whether this directory has at least one subdirectory.
    /// The client uses this to decide whether to show an expand chevron.
    #[serde(default)]
    pub has_subdirs: bool,
}

/// The directory listing response.
#[derive(Debug, Clone, Serialize)]
pub struct BrowseResponse {
    pub path: String,
    pub entries: Vec<FsEntry>,
}

/// Query parameters for the browse endpoints.
#[derive(Debug, Deserialize)]
pub struct BrowseParams {
    pub path: Option<String>,
    pub root: Option<String>,
}

/// List only the directory children of `path`, scoped to `root`.
/// `root` defaults to `/` when omitted.
pub fn browse_dirs(path: &str, root: Option<&str>) -> Result<BrowseResponse, String> {
    let root_path = resolve_root(root);
    let canonical_path = resolve_within_root(path, &root_path)?;

    let mut entries: Vec<FsEntry> = Vec::new();
    let mut read_dir = std::fs::read_dir(&canonical_path)
        .map_err(|e| format!("读取目录失败: {e}"))?;
    while let Some(entry) = read_dir.next().transpose().map_err(|e| format!("读取目录项失败: {e}"))? {
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            // Check if this subdirectory itself has any subdirectories.
            let has_subdirs = entry.path().read_dir().ok().is_some_and(|mut rd|
                rd.any(|e| e.ok().is_some_and(|e| e.file_type().ok().is_some_and(|t| t.is_dir())))
            );
            entries.push(FsEntry { name, is_dir: true, has_subdirs });
        }
    }
    entries.sort_by_key(|a| a.name.to_lowercase());
    Ok(BrowseResponse {
        path: canonical_path.to_string_lossy().to_string(),
        entries,
    })
}

/// Resolve the root directory. Defaults to `/`.
fn resolve_root(root: Option<&str>) -> PathBuf {
    match root {
        Some(r) if !r.is_empty() => PathBuf::from(r),
        _ => PathBuf::from("/"),
    }
}

/// Resolve `path` relative to `root` and validate it stays within `root`.
fn resolve_within_root(path: &str, root: &Path) -> Result<PathBuf, String> {
    let root_canonical = root
        .canonicalize()
        .unwrap_or_else(|_| root.to_path_buf());

    let target = if path.is_empty() || path == "." || path == "/" {
        root_canonical.clone()
    } else {
        let joined = root_canonical.join(path.trim_start_matches('/'));
        joined.canonicalize().map_err(|_| format!("路径不存在或无法访问: {path}"))?
    };

    if !target.starts_with(&root_canonical) {
        return Err("路径超出根目录范围".to_string());
    }

    if !target.is_dir() {
        return Err(format!("不是目录: {}", target.display()));
    }

    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_dirs_accessible_path() {
        // Use the home directory itself, which is always accessible.
        let home = std::env::var("HOME").expect("HOME must be set in tests");
        let result = browse_dirs(&home, None);
        assert!(result.is_ok(), "expected ok, got {result:?}");
        let resp = result.unwrap();
        assert!(!resp.entries.is_empty(), "home dir should have entries");
        let canonical = std::fs::canonicalize(&home).unwrap();
        assert_eq!(resp.path, canonical.to_string_lossy());
        // Every entry should have is_dir=true and has_subdirs should be set
        for e in &resp.entries {
            assert!(e.is_dir, "all entries should be directories");
        }
    }

    #[test]
    fn browse_dirs_nonexistent_path() {
        let result = browse_dirs("/tmp/__sebas_fs_test_nonexistent__", None);
        assert!(result.is_err(), "expected error for nonexistent path");
    }

    #[test]
    fn browse_dirs_file_path_rejected() {
        let result = browse_dirs("/dev/null", None);
        assert!(result.is_err(), "expected error for file path");
    }

    #[test]
    fn browse_dirs_rejects_traversal() {
        // A path outside root should be rejected. Use a root that /etc cannot escape.
        let result = browse_dirs("/etc", Some("/tmp"));
        assert!(result.is_err(), "traversal attempt should be rejected");
    }

    #[test]
    fn browse_dirs_entries_are_sorted() {
        let home = std::env::var("HOME").expect("HOME must be set in tests");
        let resp = browse_dirs(&home, None).unwrap();
        let names: Vec<&str> = resp.entries.iter().map(|e| e.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        assert_eq!(names, sorted, "entries must be sorted alphabetically");
    }

    #[test]
    fn browse_dirs_has_subdirs_flag() {
        // /usr should have subdirectories.
        let resp = browse_dirs("/usr", None).unwrap();
        let has_subdirs = resp.entries.iter().any(|e| e.has_subdirs);
        assert!(has_subdirs, "at least one entry in /usr should have subdirs");
    }
}