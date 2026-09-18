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
            // add-system-dir-denylist：命中名单的子目录不出现在树里（体验
            // 粗滤，与注册执法层同一 [`is_system_dir`] 判定——漏过由注册层
            // 兜底，两层语义同源不矛盾）。join 后判定，symlink 别名同样吸收。
            if is_system_dir(&entry.path()) {
                continue;
            }
            // Check if this subdirectory itself has any subdirectories.
            let has_subdirs = entry.path().read_dir().ok().is_some_and(|mut rd| {
                rd.any(|e| {
                    e.ok()
                        .is_some_and(|e| e.file_type().ok().is_some_and(|t| t.is_dir()))
                })
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

// ---- 内置系统目录屏蔽名单（add-system-dir-denylist）----

/// Unix 名单（spec 逐条枚举；`/lib*` 以条目模式表达为 lib/lib32/lib64/libx32）。
/// 刻意不含 `/opt` `/srv` `/mnt` `/media`——它们是合法项目位置。
#[cfg(not(windows))]
const SYSTEM_DIR_DENYLIST: &[&str] = &[
    "/", "/bin", "/sbin", "/boot", "/dev", "/etc", "/lib", "/lib32", "/lib64", "/libx32", "/proc",
    "/sys", "/usr", "/var", "/run", "/root", "/home", "/tmp",
];

/// Windows 名单（spec）：盘符根不进名单（[`is_drive_root`] 按模式判定）；
/// 无盘符前缀的卷根杂项目录按末段目录名匹配（每个卷的根下都有）。
/// 比较大小写不敏感。
#[cfg(windows)]
const SYSTEM_DIR_DENYLIST: &[&str] = &[
    r"C:\Windows",
    r"C:\Program Files",
    r"C:\Program Files (x86)",
    r"C:\ProgramData",
    r"C:\Users",
    // 无根条目：按候选解析形的末段目录名匹配。
    "System Volume Information",
    "$Recycle.Bin",
];

/// 一条名单：绝对条目存解析形（两侧同规范比较）；Windows 无根条目存目录名
/// （`by_name`，按候选末段匹配）。名单内置固定，无 config 扩展（Non-goals）。
struct DenyEntry {
    path: PathBuf,
    #[cfg(windows)]
    by_name: bool,
}

/// 名单构建（首次使用时逐条 canonicalize，17+ 条 syscall 进程一次）：解析成功
/// 存解析形——吸收平台目录别名（macOS `/tmp` → `/private/tmp`、`/lib` →
/// `/usr/lib` symlink）且候选侧的别名解析自动同域；失败保留字面形（不存在的
/// 系统目录候选也命中不了，无损失）。
fn denylist_entries() -> &'static [DenyEntry] {
    static ENTRIES: std::sync::OnceLock<Vec<DenyEntry>> = std::sync::OnceLock::new();
    ENTRIES.get_or_init(|| {
        SYSTEM_DIR_DENYLIST
            .iter()
            .map(|raw| {
                #[cfg(windows)]
                let by_name = !Path::new(raw).is_absolute();
                #[cfg(windows)]
                if by_name {
                    return DenyEntry {
                        path: PathBuf::from(raw),
                        by_name,
                    };
                }
                let resolved = std::fs::canonicalize(raw).unwrap_or_else(|_| PathBuf::from(raw));
                #[cfg(windows)]
                // Windows 比较域：dunce 普通形（verbatim 前缀判等恒假），
                // 小写比较在匹配函数里做。
                let resolved = dunce::simplified(&resolved).to_path_buf();
                DenyEntry {
                    path: resolved,
                    #[cfg(windows)]
                    by_name,
                }
            })
            .collect()
    })
}

/// Windows 盘符根模式判定（spec：drive roots 按模式而非枚举）：解析形仅剩
/// 盘符前缀分量（`C:\`）即命中。UNC 根（`\\server\share`）不是盘符根，不拦。
#[cfg(windows)]
fn is_drive_root(resolved: &Path) -> bool {
    use std::path::{Component, Prefix};
    resolved.parent().is_none()
        && resolved.has_root()
        && matches!(
            resolved.components().next(),
            Some(Component::Prefix(pc))
                if matches!(pc.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        )
}

/// 候选解析形与单条名单的比较（精确相等——只拦名单目录本身，子树放行）。
#[cfg(not(windows))]
fn matches_deny_entry(resolved: &Path, entry: &DenyEntry) -> bool {
    resolved == entry.path
}

/// Windows：两侧 dunce 普通形后按小写字符串比较（std 的组件比较不做 case
/// fold）；无根条目按候选末段目录名匹配（大小写不敏感）。
#[cfg(windows)]
fn matches_deny_entry(resolved: &Path, entry: &DenyEntry) -> bool {
    let lower = |p: &Path| p.to_string_lossy().to_lowercase();
    if entry.by_name {
        let last = resolved
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase());
        return last == Some(lower(&entry.path));
    }
    lower(resolved) == lower(&entry.path)
}

/// 候选路径是否解析到内置系统目录名单上（add-system-dir-denylist 原语）。
///
/// 两侧都先 `std::fs::canonicalize` 再比较：候选经真实路径解析（`..` 段、
/// 别名与 symlink 全部还原），名单条目在首次构建时同样解析——平台变体自动
/// 吸收。**精确匹配**：只拦名单目录本身，子树放行（`/home`、`C:\Users`、
/// `/tmp` 之下的自建目录仍是合法项目位置）。候选不可解析（不存在等）返回
/// false——拒绝文案由调用方既有存在性分支负责，这里不抢。
///
/// 消费方：`api.rs` 注册执法链（containment 之后）、[`browse_dirs`] 条目粗滤、
/// 主 crate 启动告警（workspace root 过宽提示）。判定输入是真实路径，测试
/// 一律用 tempdir，勿以真实系统目录做写操作。
pub fn is_system_dir(path: &Path) -> bool {
    let resolved = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => return false,
    };
    #[cfg(windows)]
    let resolved = dunce::simplified(&resolved).to_path_buf();
    #[cfg(windows)]
    if is_drive_root(&resolved) {
        return true;
    }
    denylist_entries()
        .iter()
        .any(|entry| matches_deny_entry(&resolved, entry))
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
        assert_eq!(second.path, dunce::simplified(&sub).to_string_lossy());
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
        assert!(
            browse_dirs("sub/..", None, dir.path()).is_ok(),
            "mid-path .. back inside root is legitimate"
        );
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
        assert!(!within_workspace_root(
            &dir.path().join("__ghost__"),
            dir.path()
        ));
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
        assert!(
            !within_workspace_root(&link, dir.path()),
            "symlink 逃逸必须越界"
        );

        let root_link = dir.path().join("root_link");
        std::os::unix::fs::symlink(dir.path(), &root_link).unwrap();
        assert!(
            within_workspace_root(&sub, &root_link),
            "root symlink 解析真实根"
        );
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
        let err = browse_dirs("", Some(other.path().to_str().unwrap()), dir.path()).unwrap_err();
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
        assert_eq!(
            normalize_windows(r"\\?\UNC\server\share/x"),
            r"\\server\share\x"
        );
        assert_eq!(normalize_windows("/bin"), r"\bin");
        assert_eq!(normalize_windows("."), ".");
    }

    #[test]
    fn canonicalize_plain_strips_verbatim_and_round_trips() {
        let (dir, _sub) = dir_with_sub();
        let plain = canonicalize_plain(dir.path()).expect("canonicalize");
        assert!(
            !plain.starts_with(r"\\?\"),
            "verbatim prefix leaked: {plain}"
        );
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
        assert_eq!(
            stored_path_in_workspace_root(&stored_sub, &ghost_root),
            None
        );
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

    // ---- 内置系统目录屏蔽名单（add-system-dir-denylist）----

    #[cfg(unix)]
    #[test]
    fn system_dir_denylist_hits_exact_entries_and_subtree_passes() {
        use std::os::unix::fs::symlink;

        // 命中：真实存在的名单目录（用 /tmp 自身——存在性由沙箱保证；其余
        // 名单目录在容器里未必都在，不逐一依赖）。
        assert!(is_system_dir(Path::new("/tmp")), "/tmp must hit");
        assert!(is_system_dir(Path::new("/")), "root must hit");

        // 子树放行：tempdir 名单目录（/tmp）之下的自建目录不命中——spec
        // 「只拦名单目录本身」。
        let (dir, sub) = dir_with_sub();
        assert!(!is_system_dir(dir.path()), "tempdir under /tmp must pass");
        assert!(!is_system_dir(&sub), "subtree of a denylisted dir passes");

        // `..` 段解析后再比较：/tmp/../tmp 解析回名单目录本身 → 命中。
        assert!(
            is_system_dir(Path::new("/tmp/../tmp")),
            ".. segments resolve before matching"
        );

        // symlink 指向名单目录：解析真实目标后命中。
        let link = dir.path().join("to_tmp");
        symlink("/tmp", &link).unwrap();
        assert!(
            is_system_dir(&link),
            "symlink onto denylist resolves to hit"
        );

        // 不存在路径：fail 路径返回 false（不在此拒，交给调用方存在分支）。
        assert!(!is_system_dir(&dir.path().join("__ghost__")));

        // tempdir 子树（/tmp 下）恒不误伤：browse 语义的既有测试形态依赖它。
        assert!(!is_system_dir(
            dir.path().join("sub").join("deep").as_path()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn system_dir_denylist_tmp_resolves_to_private_variant() {
        // macOS /tmp → /private/tmp 变体由「两侧解析」机制吸收：这里断言
        // 解析形确实与字面形可能不同时仍命中（/tmp 本身存在，canonicalize
        // 必成功——若环境把 /tmp 链到别处，解析形进名单照样命中）。
        let literal = PathBuf::from("/tmp");
        let resolved = std::fs::canonicalize(&literal).unwrap();
        assert!(is_system_dir(&resolved), "resolved form of /tmp must hit");
        // 字面形与解析形在 Linux 一致时本断言即退化为主用例，无损失。
        assert!(is_system_dir(&literal));
    }

    #[test]
    fn system_dir_denylist_unresolvable_candidate_is_false() {
        let (dir, _sub) = dir_with_sub();
        assert!(
            !is_system_dir(&dir.path().join("no").join("such").join("dir")),
            "unresolvable candidate is not a denylist hit"
        );
    }

    #[cfg(windows)]
    #[test]
    fn system_dir_denylist_windows_case_insensitive_and_drive_root() {
        assert!(
            is_system_dir(Path::new(r"c:\WINDOWS")),
            "case-insensitive hit"
        );
        assert!(is_system_dir(Path::new(r"C:\windows")));
        assert!(is_system_dir(Path::new(r"C:\Program Files")));
        assert!(is_system_dir(Path::new(r"c:\PROGRAM FILES")));
        assert!(is_system_dir(Path::new(r"C:\")), "drive root by pattern");
        // 模式判定的纯函数面：任意盘符（含不存在的盘）都按根分量命中。
        assert!(is_drive_root(Path::new(r"Q:\")));
        assert!(!is_drive_root(Path::new(r"C:\Users")));
        // 子树放行。
        assert!(!is_system_dir(Path::new(r"C:\Users\someone\code")));
        assert!(!is_system_dir(Path::new(r"C:\Windows\System32")));
        // 不存在的路径 fail 路径。
        assert!(!is_system_dir(Path::new(r"C:\__no_such_dir__")));
    }

    #[test]
    #[cfg(unix)] // 造「解析到名单目录」的条目要靠 symlink 到 /tmp、/etc
    fn browse_dirs_omits_denylisted_children_others_unchanged() {
        // 名单形子目录造在 tempdir 里（tempdir 在 /tmp 之下，其**子**目录
        // 按精确匹配语义不命中——所以这里造的是解析形恰为名单形的东西：
        // unix 上唯一的名单形子目录是 /tmp 自身，它作为 tempdir 的孩子
        // 无法自然出现；改用 symlink 造「解析到名单目录」的条目，这正是
        // 过滤要拦的形态（列表条目解析到名单即隐藏，无论字面名是什么）。
        let (dir, _sub) = dir_with_sub();
        std::os::unix::fs::symlink("/tmp", dir.path().join("usr")).unwrap();
        std::os::unix::fs::symlink("/etc", dir.path().join("etc")).unwrap();

        let resp = browse_dirs("", None, dir.path()).unwrap();
        let names: Vec<&str> = resp.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(
            !names.contains(&"usr") && !names.contains(&"etc"),
            "denylisted-resolving children must be filtered: {names:?}"
        );
        // 其余条目照旧 + round-trip 不变。
        assert!(names.contains(&"sub") && names.contains(&"zeta"));
        let child = format!("{}/sub", resp.path);
        let second = browse_dirs(&child, None, dir.path()).unwrap();
        assert!(
            second.entries.iter().any(|e| e.name == "deep"),
            "round-trip holds"
        );
    }

    #[test]
    fn browse_dirs_tempdir_children_are_never_denylist_hits() {
        // 粗滤不误伤：tempdir（/tmp 下）的普通子目录必须照常列出。
        let (dir, _sub) = dir_with_sub();
        let resp = browse_dirs("", None, dir.path()).unwrap();
        let names: Vec<&str> = resp.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"sub") && names.contains(&"zeta"),
            "{names:?}"
        );
    }

    // ---- 名单补强（review 轮新增：内容钉死 / 全条目命中 / 文件候选 / 显式 root）----

    #[cfg(unix)]
    #[test]
    fn system_dir_denylist_unix_entries_match_spec_exactly() {
        // 钉死名单内容 = spec 逐条枚举（防静默增删：删条目会让对应系统目录
        // 可注册，加条目会误伤合法位置）。顺序也按 spec，diff 可读。
        let expected: &[&str] = &[
            "/", "/bin", "/sbin", "/boot", "/dev", "/etc", "/lib", "/lib32", "/lib64", "/libx32",
            "/proc", "/sys", "/usr", "/var", "/run", "/root", "/home", "/tmp",
        ];
        assert_eq!(SYSTEM_DIR_DENYLIST, expected);
    }

    #[cfg(unix)]
    #[test]
    fn system_dir_denylist_every_existing_entry_hits() {
        // 逐条断言「本机真实存在」的名单条目必命中（容器里不存在的条目
        // canonicalize 失败，候选侧同样解析不了，跳过而非弱化）。merged-usr
        // 发行版上 /bin、/lib 是指向 /usr/* 的 symlink——两侧同解析机制必须
        // 让字面条目经解析形照样命中。
        for raw in SYSTEM_DIR_DENYLIST {
            let p = Path::new(raw);
            if std::fs::canonicalize(p).is_ok() {
                assert!(is_system_dir(p), "existing denylist entry {raw} must hit");
            }
        }
        // 尾随斜杠与 `..` 归位后仍命中：`/tmp/` 归位 `/tmp`；`/tmp/..` 归位
        // `/`（root 条目经遍历形命中）。
        assert!(
            is_system_dir(Path::new("/tmp/")),
            "trailing slash normalizes"
        );
        assert!(
            is_system_dir(Path::new("/tmp/..")),
            "/tmp/.. resolves onto /"
        );
    }

    #[cfg(unix)]
    #[test]
    fn system_dir_denylist_leaves_deliberate_roots_alone() {
        // spec 明文刻意不拦的四个根：存在与否都不命中（不存在时
        // canonicalize 失败 → false，同一断言两种形态都过）。
        for p in ["/opt", "/srv", "/mnt", "/media"] {
            assert!(!is_system_dir(Path::new(p)), "{p} must stay registrable");
        }
    }

    #[test]
    fn system_dir_denylist_file_candidates_never_hit() {
        // 文件（非目录）候选不命中：精确匹配按完整路径，与末段名无关——
        // tempdir 里与名单目录同名的**文件**照常放行。注册侧这类候选由既有
        // is_dir 分支以「路径不是目录」拒绝，名单不抢戏（也不在 Windows 的
        // by-name 条目上误伤同名文件——unix 无 by-name 条目，此处钉行为面）。
        let dir = tempfile::tempdir().unwrap();
        for name in ["usr", "etc", "tmp"] {
            let f = dir.path().join(name);
            std::fs::write(&f, b"x").unwrap();
            assert!(!is_system_dir(&f), "file named {name} must not hit");
        }
    }

    #[cfg(unix)]
    #[test]
    fn browse_dirs_filters_denylisted_children_with_explicit_root() {
        // root 参数与 workspace root 组合下过滤语义一致：显式 root 的条目
        // 产出走同一 is_system_dir 滤除，非名单条目与 round-trip 不受影响。
        let (dir, _sub) = dir_with_sub();
        std::os::unix::fs::symlink("/tmp", dir.path().join("usr")).unwrap();
        let resp = browse_dirs("", Some(dir.path().to_str().unwrap()), dir.path()).unwrap();
        let names: Vec<&str> = resp.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(
            !names.contains(&"usr"),
            "explicit root must filter too: {names:?}"
        );
        assert!(names.contains(&"sub") && names.contains(&"zeta"));
    }
}
