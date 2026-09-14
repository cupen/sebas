//! Directory browser for the Add Project dialog's path picker.
//!
//! Exposes `GET /api/fs/browse-dirs?path=<path>&root=<root>` which returns the
//! immediate child directories of a path. The browse root — the scope every
//! request resolves within — is the workspace root by default; an explicit
//! `root` query parameter is honoured only inside it (add-workspace-root).
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

/// 工作区根目录范围判定原语（add-workspace-root）。
///
/// `candidate` 与 `root` 都先 canonicalize 再做逐分量前缀比较（与 `safe_path`
/// 的越界检查同一语义，symlink 解析后的真实根参与比较）。任一侧 canonicalize
/// 失败即不在范围内（fail-closed）：候选不可解析 → 越界；root 不可解析 →
/// 一切候选越界。workspace root 恒存在（不存在「未启用约束」的运行形态）。
pub fn within_workspace_root(candidate: &Path, root: &Path) -> bool {
    let resolved = match std::fs::canonicalize(candidate) {
        Ok(p) => p,
        Err(_) => return false,
    };
    std::fs::canonicalize(root)
        .map(|r| resolved.starts_with(&r))
        .unwrap_or(false)
}

/// 已 canonical 存储值的执法基座（add-workspace-root 2.2/2.3）：把 workspace
/// root 解析成与存储值同域的普通形。注册/会话绑定落库的是 `canonicalize_plain`
/// 的无 verbatim 前缀形，而 `std::fs::canonicalize` 在 Windows 产出 `\\?\`
/// verbatim 形——`VerbatimDisk` 与 `Disk` 前缀分量判等恒 false，比较前必须拉回
/// 同一普通形。`None` = root 不可解析（fail-closed 语义由调用方落地：列表全部
/// 隐藏 + warn、会话面按越界拒绝）。
pub fn workspace_root_prefix(root: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(root)
        .ok()
        .map(|r| dunce::simplified(&r).to_path_buf())
}

/// 已 canonical 存储的路径是否落在 root 前缀内（逐分量比较；不触碰文件系统，
/// 目录此后被删/移走也不改写判定输入）。与 [`within_workspace_root`] 的分工：
/// 那个用于**用户输入**（先 canonicalize 再比较），这个用于**已 canonical 的
/// 存储值**（列表过滤、会话面执法）——存储值 re-canonicalize 会把「目录被删」
/// 错判成「越界之外的不确定态」，且多一次无谓的系统调用。
pub fn stored_path_within_prefix(stored: &str, root_prefix: &Path) -> bool {
    Path::new(stored).starts_with(root_prefix)
}

/// 存储路径对 workspace root 的单步判定。`None` = root 不可解析（fail-closed：
/// 调用方按越界处理）。会话面每请求至多一次，root 解析开销可忽略；列表过滤
/// 请用 [`workspace_root_prefix`] 解析一次后逐条 [`stored_path_within_prefix`]，
/// 避免 warn 与解析按项目条数重复。
pub fn stored_path_in_workspace_root(stored: &str, root: &Path) -> Option<bool> {
    Some(stored_path_within_prefix(
        stored,
        &workspace_root_prefix(root)?,
    ))
}

/// 项目注册/回显用的规范形：canonicalize 解析真实路径后，把 Windows
/// verbatim 前缀与混合分隔符还原成普通形（projects_add 曾把 `\\?\C:\…`
/// 直接入库，前缀经 API 泄漏进 UI，且与请求侧普通路径判等永假）。
/// 范围判定不走这里——`within_workspace_root` 是两侧同规范的纯比较。
pub fn canonicalize_plain(path: &Path) -> std::io::Result<String> {
    let canonical = std::fs::canonicalize(path)?;
    let s = canonical.to_string_lossy();
    #[cfg(windows)]
    let s = normalize_windows(&s);
    #[cfg(not(windows))]
    let s = s.into_owned();
    Ok(s)
}

/// Resolve a browse request to the directory to list as `(canonical target,
/// echo form)`. The single owner of browse path semantics.
///
/// Root precedence: the request's explicit `root` beats the injected
/// `workspace_root` (which is also the no-param browse start). The root must
/// exist; there is no silent raw-path fallback (a non-canonical root would
/// weaken the bounds check below). The request `path` is normalized before
/// joining (verbatim prefixes undone, separators unified, doubled separators
/// collapsed), so any form a client echoes back resolves; an absolute request
/// replaces the root base — the historical `Path::join` semantics. The bounds
/// check is component-wise against the canonicalized root, so `..`, absolute
/// escapes, and symlinks pointing outside the root are rejected.
///
/// workspace root（add-workspace-root）：显式 `root` 必须落在 workspace root
/// 之内，否则「路径超出允许范围」（候选不可解析同罪，fail-closed）；无参
/// 请求的浏览起点就是 workspace root 本身。
pub fn safe_path(
    path: &str,
    explicit_root: Option<&str>,
    workspace_root: &Path,
) -> Result<(PathBuf, String), String> {
    let root = explicit_root
        .filter(|r| !r.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.to_path_buf());

    if explicit_root.filter(|r| !r.is_empty()).is_some()
        && !within_workspace_root(&root, workspace_root)
    {
        return Err("路径超出允许范围: root 不在 workspace root 内".to_string());
    }
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

/// List only the directory children of `path`, scoped to a root. 无参起点 =
/// workspace root（add-workspace-root：浏览起点收敛到 workspace root）。
pub fn browse_dirs(
    path: &str,
    explicit_root: Option<&str>,
    workspace_root: &Path,
) -> Result<BrowseResponse, String> {
    let (canonical_path, echo) = safe_path(path, explicit_root, workspace_root)?;

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
        // add-workspace-root：无参浏览起点 = workspace root 本身。
        let (dir, _sub) = dir_with_sub();
        let resp = browse_dirs("", None, dir.path()).unwrap();
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
        let first = browse_dirs("", None, dir.path()).unwrap();
        assert!(!first.path.contains(r"\\?\"), "echo must be simplified");

        let child_request = format!("{}/sub", first.path);
        let second = browse_dirs(&child_request, None, dir.path()).unwrap();
        assert_eq!(
            second.path,
            dunce::simplified(&sub).to_string_lossy()
        );
        assert!(second.entries.iter().any(|e| e.name == "deep"));
    }

    #[test]
    fn explicit_root_overrides_workspace_root_start() {
        // 显式 root 在 workspace root 之内 → 以它为本次浏览范围。
        let (dir, _sub) = dir_with_sub();
        let sub = dir.path().join("sub");
        let resp = browse_dirs("", Some(sub.to_str().unwrap()), dir.path()).unwrap();
        assert_eq!(resp.path, dunce::simplified(&sub).to_string_lossy());
    }

    #[test]
    fn nonexistent_root_is_an_error_not_a_silent_fallback() {
        // 显式 root 不可解析 = 越界（fail-closed，与确定越界同文案）。
        let (dir, _sub) = dir_with_sub();
        let ghost = dir.path().join("__no_such_root__");
        let result = browse_dirs("", Some(ghost.to_str().unwrap()), dir.path());
        assert!(result.is_err(), "root must exist; silent fallback is gone");
    }

    #[test]
    fn nonexistent_path_error_echoes_the_request_input() {
        let (dir, _sub) = dir_with_sub();
        let err = browse_dirs("no_such_dir", None, dir.path()).unwrap_err();
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
        let err = browse_dirs("file.txt", None, dir.path()).unwrap_err();
        assert_eq!(err, "不是目录");
    }

    #[test]
    fn traversal_rejected() {
        let (dir, _sub) = dir_with_sub();
        let err = browse_dirs("..", None, dir.path()).unwrap_err();
        assert_eq!(err, "路径超出根目录范围");
        assert!(browse_dirs("sub/..", None, dir.path()).is_ok(), "mid-path .. back inside root is legitimate");
    }

    #[test]
    fn absolute_escape_rejected() {
        let (dir, _sub) = dir_with_sub();
        let (sibling, _s) = dir_with_sub();
        let result = browse_dirs(sibling.path().to_str().unwrap(), None, dir.path());
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
        let err = browse_dirs("evil", None, dir.path()).unwrap_err();
        assert_eq!(err, "路径超出根目录范围");
    }

    #[test]
    fn entries_sorted_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("B_dir")).unwrap();
        std::fs::create_dir_all(dir.path().join("a_dir")).unwrap();
        let resp = browse_dirs("", None, dir.path()).unwrap();
        let names: Vec<&str> = resp.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a_dir", "B_dir"]);
    }

    #[test]
    fn has_subdirs_flag() {
        let (dir, _sub) = dir_with_sub();
        let resp = browse_dirs("", None, dir.path()).unwrap();
        let sub = resp.entries.iter().find(|e| e.name == "sub").unwrap();
        let zeta = resp.entries.iter().find(|e| e.name == "zeta").unwrap();
        assert!(sub.has_subdirs, "sub has a child dir");
        assert!(!zeta.has_subdirs, "zeta is empty");
    }

    // ---- workspace root 范围判定（add-workspace-root）----

    #[test]
    fn within_workspace_root_hit_miss_and_traversal() {
        let (dir, sub) = dir_with_sub();
        let (outside, _o) = dir_with_sub();

        assert!(within_workspace_root(dir.path(), dir.path()), "根自身命中");
        assert!(within_workspace_root(&sub, dir.path()), "子目录命中");
        assert!(
            !within_workspace_root(outside.path(), dir.path()),
            "树外目录不命中"
        );

        // `..` 穿越：canonicalize 先解析再比较——解析后仍在根内 → 命中，
        // 越出根 → 越界。
        let back_inside = sub.join("..");
        assert!(within_workspace_root(&back_inside, dir.path()));
        let escaped = sub.join("..").join("..");
        assert!(
            !within_workspace_root(&escaped, dir.path()),
            "`..` 越出根即越界"
        );

        // 候选不可解析 → 越界（fail-closed）。
        assert!(!within_workspace_root(&dir.path().join("__ghost__"), dir.path()));
    }

    /// symlink 逃逸：根内链接指向根外 → 越界；root 自身经 symlink 给出 →
    /// 解析为真实根后比较。Windows 建链需要特权，整段 unix 门控。
    #[cfg(unix)]
    #[test]
    fn within_workspace_root_symlink_escape_and_root_link() {
        let (dir, sub) = dir_with_sub();
        let (outside, _o) = dir_with_sub();

        let link = dir.path().join("link");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        assert!(!within_workspace_root(&link, dir.path()), "symlink 逃逸必须越界");

        let root_link = dir.path().join("root_link");
        std::os::unix::fs::symlink(dir.path(), &root_link).unwrap();
        assert!(within_workspace_root(&sub, &root_link), "root symlink 解析真实根");
    }

    #[test]
    fn unresolvable_root_rejects_every_candidate() {
        // root 不可解析 → 一切候选越界（fail-closed），包括真实存在的目录。
        let (dir, _sub) = dir_with_sub();
        let ghost_root = dir.path().join("__ghost_root__");
        assert!(!within_workspace_root(dir.path(), &ghost_root));
    }

    #[test]
    fn explicit_root_outside_workspace_root_rejected() {
        let (dir, _sub) = dir_with_sub();
        let (other, _o) = dir_with_sub();
        // 显式 root 指向 workspace root 之外 → 范围错误（而非列出内容）。
        let err = browse_dirs(
            "",
            Some(other.path().to_str().unwrap()),
            dir.path(),
        )
        .unwrap_err();
        assert_eq!(err, "路径超出允许范围: root 不在 workspace root 内");
    }

    #[test]
    fn explicit_root_inside_workspace_root_accepted() {
        let (dir, sub) = dir_with_sub();
        let resp = browse_dirs(
            sub.to_str().unwrap(),
            Some(dir.path().to_str().unwrap()),
            dir.path(),
        )
        .unwrap();
        assert!(resp.entries.iter().any(|e| e.name == "deep"));
    }

    #[test]
    fn explicit_root_bounds_are_not_widened_by_workspace_root() {
        // 显式 root 在 workspace root 内，但 path 越出该 root（仍根内）：
        // 原有的 root 越界检查仍生效——workspace root 只约束 root 选择，
        // 不放大单次请求的范围。`../zeta` 相对显式 root（sub）解析出
        // dir/zeta：真实存在、在 workspace root 内、在显式 root 外。
        let (dir, _sub) = dir_with_sub();
        let err = browse_dirs(
            "../zeta",
            Some(dir.path().join("sub").to_str().unwrap()),
            dir.path(),
        )
        .unwrap_err();
        assert_eq!(err, "路径超出根目录范围");
    }


    #[cfg(windows)]
    #[test]
    fn verbatim_explicit_root_is_accepted() {
        let (dir, _sub) = dir_with_sub();
        let verbatim = std::fs::canonicalize(dir.path()).unwrap();
        let resp = browse_dirs("", Some(verbatim.to_str().unwrap()), dir.path()).unwrap();
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

    #[test]
    fn canonicalize_plain_strips_verbatim_and_round_trips() {
        let (dir, _sub) = dir_with_sub();
        let plain = canonicalize_plain(dir.path()).expect("canonicalize");
        assert!(!plain.starts_with(r"\\?\"), "verbatim prefix leaked: {plain}");
        assert!(Path::new(&plain).is_dir());
        // 注册后再 canonicalize 同一普通路径必须得到同一普通形（分隔符不漂移）。
        let again = canonicalize_plain(Path::new(&plain)).expect("re-canonicalize");
        assert_eq!(plain, again);
    }

    // ---- 已 canonical 存储值的范围判定（add-workspace-root 2.2/2.3）----

    #[test]
    fn stored_path_prefix_hit_miss_and_unresolvable_root() {
        let (dir, sub) = dir_with_sub();
        let (outside, _o) = dir_with_sub();

        // 存储值取 canonicalize_plain 的普通形（注册落库形态）；root 单独
        // canonicalize——Windows 上带 verbatim 前缀，正是普通形比较要防的错位。
        let stored_sub = canonicalize_plain(&sub).unwrap();
        let stored_outside = canonicalize_plain(outside.path()).unwrap();
        assert_eq!(
            stored_path_in_workspace_root(&stored_sub, dir.path()),
            Some(true),
            "界内存储路径命中"
        );
        assert_eq!(
            stored_path_in_workspace_root(&stored_outside, dir.path()),
            Some(false),
            "树外存储路径不命中"
        );

        // root 缺失 → None（fail-closed 交调用方），即使存储路径真实存在。
        let ghost_root = dir.path().join("__ghost_root__");
        assert_eq!(stored_path_in_workspace_root(&stored_sub, &ghost_root), None);
    }

    #[test]
    fn stored_path_prefix_survives_deleted_candidate_dir() {
        // 存储值不再 canonicalize：目录此后被删只影响可达性，不改写「注册时
        // 在根内」的判定输入（围栏不锁垃圾——会话仍可 close/archive 清理）。
        let (dir, _sub) = dir_with_sub();
        let doomed = dir.path().join("doomed");
        std::fs::create_dir_all(&doomed).unwrap();
        let stored = canonicalize_plain(&doomed).unwrap();
        std::fs::remove_dir_all(&doomed).unwrap();
        assert!(stored_path_in_workspace_root(&stored, dir.path()) == Some(true));
    }
}
