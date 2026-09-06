//! Directory browser for the Add Project dialog's path picker.
//!
//! Exposes `GET /api/fs/browse-dirs?path=<path>&root=<root>` which returns the
//! immediate child directories of a path. The browse root — the scope every
//! request resolves within — is the server's configured work directory by
//! default; an explicit `root` query parameter overrides it
//! (add-webui-picker-workdir-start).
//!
//! Path semantics are owned by [`safe_path`]: it guarantees the `path` echoed
//! in a response round-trips as a later request (Windows verbatim prefixes and
//! mixed separators are normalized away) and rejects anything that resolves
//! outside the root.

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

/// Resolve a browse request to the directory to list as `(canonical target,
/// echo form)`. The single owner of browse path semantics.
///
/// Root precedence: the request's explicit `root` beats the server-injected
/// `server_default_root` (the configured work dir); when both are missing this
/// is a hard error — the old `/` filesystem-root default is gone. The root
/// must exist; there is no silent raw-path fallback (a non-canonical root
/// would weaken the bounds check below). The request `path` is normalized
/// before joining (verbatim prefixes undone, separators unified, doubled
/// separators collapsed), so any form a client echoes back resolves; an
/// absolute request replaces the root base — the historical `Path::join`
/// semantics. The bounds check is component-wise against the canonicalized
/// root, so `..`, absolute escapes, and symlinks pointing outside the root
/// are rejected.
pub fn safe_path(
    path: &str,
    explicit_root: Option<&str>,
    server_default_root: Option<&Path>,
) -> Result<(PathBuf, String), String> {
    let root = explicit_root
        .filter(|r| !r.is_empty())
        .map(PathBuf::from)
        .or_else(|| server_default_root.map(Path::to_path_buf))
        .ok_or_else(|| "缺少浏览根目录: 请求未带 root 且服务端未配置 work dir".to_string())?;
    let root_canonical = std::fs::canonicalize(&root)
        .map_err(|e| format!("根目录不存在或无法访问: {} ({e})", root.display()))?;

    #[cfg(windows)]
    let requested = normalize_windows(path);
    // unix 同样保住「绝对请求替换 root 基座」的 Path::join 语义：只去尾部
    // 冗余斜杠（"/" 整体视作 root），绝不剥前导 /——剥了会把绝对请求错误
    // 拼成 root 内相对路径，越界请求也只会误报「不存在」而非「超出范围」。
    #[cfg(not(windows))]
    let requested = {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            String::new()
        } else {
            trimmed.to_string()
        }
    };

    let target = if requested.is_empty() || requested == "." {
        root_canonical.clone()
    } else {
        let joined = root_canonical.join(&requested);
        joined
            .canonicalize()
            .map_err(|_| format!("路径不存在或无法访问: {path}"))?
    };

    if !target.starts_with(&root_canonical) {
        return Err("路径超出根目录范围".to_string());
    }
    if !target.is_dir() {
        // 错误体不回显服务端解析后的路径（webui 暴露到非 loopback 时会泄露
        // 目录布局）；完整路径只进日志。
        tracing::warn!(dir = %target.display(), "browse-dirs: path is not a directory");
        return Err("不是目录".to_string());
    }

    let echo = dunce::simplified(&target).to_string_lossy().to_string();
    Ok((target, echo))
}

/// List only the directory children of `path`, scoped to a root.
pub fn browse_dirs(
    path: &str,
    explicit_root: Option<&str>,
    server_default_root: Option<&Path>,
) -> Result<BrowseResponse, String> {
    let (canonical_path, echo) = safe_path(path, explicit_root, server_default_root)?;

    let mut entries: Vec<FsEntry> = Vec::new();
    let mut read_dir =
        std::fs::read_dir(&canonical_path).map_err(|e| format!("读取目录失败: {e}"))?;
    while let Some(entry) = read_dir
        .next()
        .transpose()
        .map_err(|e| format!("读取目录项失败: {e}"))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            // Check if this subdirectory itself has any subdirectories.
            let has_subdirs = entry.path().read_dir().ok().is_some_and(|mut rd| {
                rd.any(|e| e.ok().is_some_and(|e| e.file_type().ok().is_some_and(|t| t.is_dir())))
            });
            entries.push(FsEntry {
                name,
                is_dir: true,
                has_subdirs,
            });
        }
    }
    entries.sort_by_key(|a| a.name.to_lowercase());
    Ok(BrowseResponse {
        path: echo,
        entries,
    })
}

/// Windows request-path normalization: undo verbatim prefixes (they skip
/// Win32 separator normalization entirely, so a `/` past `\\?\` is a hard
/// failure — the exact bug this change fixes), unify separators, and collapse
/// doubled separators while preserving the leading `\\` a UNC path needs.
#[cfg(windows)]
fn normalize_windows(path: &str) -> String {
    let stripped = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.to_string()
    };
    let mut out = String::with_capacity(stripped.len());
    for c in stripped.chars() {
        let c = if c == '/' { '\\' } else { c };
        if c == '\\' && out.ends_with('\\') && out.len() != 1 {
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tempdir shaped for browse tests: `sub/` (has a subdir), `zeta/`
    /// (empty), and `file.txt` (must never be listed).
    fn dir_with_sub() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub").join("deep")).unwrap();
        std::fs::create_dir_all(dir.path().join("zeta")).unwrap();
        std::fs::write(dir.path().join("file.txt"), b"x").unwrap();
        let sub = dir.path().join("sub");
        (dir, sub)
    }

    #[test]
    fn browse_dirs_lists_default_root_without_root_param() {
        let (dir, _sub) = dir_with_sub();
        let resp = browse_dirs("", None, Some(dir.path())).unwrap();
        assert_eq!(resp.path, dunce::simplified(dir.path()).to_string_lossy());
        let names: Vec<&str> = resp.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"sub") && names.contains(&"zeta"));
        assert!(!names.contains(&"file.txt"), "only directories are listed");
    }

    /// 往返契约（webui spec「browse-dirs path round-trips on expand」）：
    /// 回显 path 拼 `/` + 子目录名后原样回传必须 200。Windows 上这正是
    /// `\\?\D:\tmp\x/sub` 混合分隔符形态——本 change 修复的展开报错 bug。
    #[test]
    fn echoed_path_round_trips_with_forward_slash_join() {
        let (dir, sub) = dir_with_sub();
        let first = browse_dirs("", None, Some(dir.path())).unwrap();
        assert!(!first.path.contains(r"\\?\"), "echo must be simplified");

        let child_request = format!("{}/sub", first.path);
        let second = browse_dirs(&child_request, None, Some(dir.path())).unwrap();
        assert_eq!(
            second.path,
            dunce::simplified(&sub).to_string_lossy()
        );
        assert!(second.entries.iter().any(|e| e.name == "deep"));
    }

    #[test]
    fn explicit_root_overrides_server_default() {
        let (dir, _sub) = dir_with_sub();
        let (other, _other_sub) = dir_with_sub();
        let resp = browse_dirs("", Some(other.path().to_str().unwrap()), Some(dir.path())).unwrap();
        assert_eq!(resp.path, dunce::simplified(other.path()).to_string_lossy());
        assert!(resp.path != dunce::simplified(dir.path()).to_string_lossy());
    }

    #[test]
    fn missing_root_is_a_hard_error() {
        // 显式与默认皆缺 → 硬错误（不再自造 `/` 文件系统根默认）。
        assert!(safe_path("", None, None).is_err());
        assert!(browse_dirs("", None, None).is_err());
    }

    #[test]
    fn nonexistent_root_is_an_error_not_a_silent_fallback() {
        let (dir, _sub) = dir_with_sub();
        let ghost = dir.path().join("__no_such_root__");
        let result = browse_dirs("", Some(ghost.to_str().unwrap()), Some(dir.path()));
        assert!(result.is_err(), "root must exist; silent fallback is gone");
    }

    #[test]
    fn nonexistent_path_error_echoes_the_request_input() {
        let (dir, _sub) = dir_with_sub();
        let err = browse_dirs("no_such_dir", None, Some(dir.path())).unwrap_err();
        // 回显的是调用方入参（而非服务端解析后的绝对路径）。
        assert!(err.contains("no_such_dir"), "{err}");
        assert!(
            !err.contains(&dunce::simplified(dir.path()).to_string_lossy().into_owned()),
            "resolved server path must not leak into the error body: {err}"
        );
    }

    #[test]
    fn file_path_is_rejected_without_resolved_path_in_error() {
        let (dir, _sub) = dir_with_sub();
        let err = browse_dirs("file.txt", None, Some(dir.path())).unwrap_err();
        assert_eq!(err, "不是目录");
    }

    #[test]
    fn traversal_rejected() {
        let (dir, _sub) = dir_with_sub();
        let err = browse_dirs("..", None, Some(dir.path())).unwrap_err();
        assert_eq!(err, "路径超出根目录范围");
        assert!(browse_dirs("sub/..", None, Some(dir.path())).is_ok(), "mid-path .. back inside root is legitimate");
    }

    #[test]
    fn absolute_escape_rejected() {
        let (dir, _sub) = dir_with_sub();
        let (sibling, _s) = dir_with_sub();
        let result = browse_dirs(sibling.path().to_str().unwrap(), None, Some(dir.path()));
        assert_eq!(result.unwrap_err(), "路径超出根目录范围");
    }

    /// symlink 指向树外：canonicalize 解析真实目标后按分量比较，越界即拒。
    /// Windows 建链需要特权，仅 unix 跑（Windows 的拒绝路径与 unix 同一
    /// canonicalize + starts_with 代码）。
    #[cfg(unix)]
    #[test]
    fn symlink_escape_rejected() {
        let (dir, _sub) = dir_with_sub();
        let (outside, _o) = dir_with_sub();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("evil")).unwrap();
        let err = browse_dirs("evil", None, Some(dir.path())).unwrap_err();
        assert_eq!(err, "路径超出根目录范围");
    }

    #[test]
    fn entries_sorted_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("B_dir")).unwrap();
        std::fs::create_dir_all(dir.path().join("a_dir")).unwrap();
        let resp = browse_dirs("", None, Some(dir.path())).unwrap();
        let names: Vec<&str> = resp.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a_dir", "B_dir"]);
    }

    #[test]
    fn has_subdirs_flag() {
        let (dir, _sub) = dir_with_sub();
        let resp = browse_dirs("", None, Some(dir.path())).unwrap();
        let sub = resp.entries.iter().find(|e| e.name == "sub").unwrap();
        let zeta = resp.entries.iter().find(|e| e.name == "zeta").unwrap();
        assert!(sub.has_subdirs, "sub has a child dir");
        assert!(!zeta.has_subdirs, "zeta is empty");
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_explicit_root_is_accepted() {
        let (dir, _sub) = dir_with_sub();
        let verbatim = std::fs::canonicalize(dir.path()).unwrap();
        let resp = browse_dirs("", Some(verbatim.to_str().unwrap()), None).unwrap();
        assert_eq!(resp.path, dunce::simplified(dir.path()).to_string_lossy());
    }

    #[cfg(windows)]
    #[test]
    fn normalize_windows_unifies_and_collapses_separators() {
        assert_eq!(normalize_windows(r"\\?\D:\/bin"), r"D:\bin");
        assert_eq!(normalize_windows("D:/x//y"), r"D:\x\y");
        assert_eq!(normalize_windows(r"\\server\share/x"), r"\\server\share\x");
        assert_eq!(normalize_windows(r"\\?\UNC\server\share/x"), r"\\server\share\x");
        assert_eq!(normalize_windows("/bin"), r"\bin");
        assert_eq!(normalize_windows("."), ".");
    }
}
