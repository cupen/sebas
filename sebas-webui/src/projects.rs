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
    /// Stable wire identifier (`proj-<12hex>`), deterministically derived
    /// from the canonicalised path (workbench-agent-wire-fix D2). Survives
    /// restarts and registry rebuilds; the wire never carries the raw path.
    /// `#[serde(default)]` backfills entries persisted by an older registry.
    #[serde(default)]
    pub id: String,
    pub path: String,
    pub name: String,
    pub added_at: u64,
    /// The agent id most recently used to create a session under this
    /// project (project-level default agent, workbench-agent-wire-fix D5).
    /// `None` = the operator has not created a session here yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
    /// Git branch read lazily; refreshed at most once per `BRANCH_TTL_SECS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Unix seconds of the last branch probe (0 = never).
    #[serde(default)]
    pub branch_at: u64,
    /// 项目所在的**执行节点**（add-remote-execution-node 3.2）。
    ///
    /// 项目身份是 `(节点, 路径)`：同一个路径在两台机器上是**两个项目**，因此 id 也
    /// 必须随节点不同（见 [`project_id_for_on`]）。`#[serde(default)]` 让旧注册表
    /// 自动回填为 [`LOCAL_NODE_ID`]——那就是迁移，不需要额外的迁移脚本。
    #[serde(default = "default_node_id")]
    pub node_id: String,
}

/// 本机节点的标识（旧数据与本地注册都用它）。
pub const LOCAL_NODE_ID: &str = "local";

fn default_node_id() -> String {
    LOCAL_NODE_ID.to_string()
}

impl ProjectEntry {
    /// 是否注册在本机节点上。
    ///
    /// 只有本机项目才能做本地文件系统操作（`is_accessible` / 分支探测 / 目录浏览）；
    /// 远端项目的路径可用性由**节点在 spawn 时**判定（3.3），主控不做本地 stat。
    pub fn is_local(&self) -> bool {
        self.node_id == LOCAL_NODE_ID
    }
}

/// Deterministic project id: `proj-` + first 12 hex of SHA-256 over the
/// canonicalised path. Same path → same id across restarts and registry
/// rebuilds; no allocator state to persist.
pub fn project_id_for(canonical_path: &str) -> String {
    project_id_for_on(LOCAL_NODE_ID, canonical_path)
}

/// 按 `(节点, 路径)` 派生确定性 id。
///
/// **本机保持既有公式**：已注册的本机项目 id 必须原样不变（它们已经进了持久注册表与
/// 前端 URL，改公式等于把既有项目变成孤儿）。远端节点则把节点名拌进哈希，于是同一
/// 路径在两台机器上得到两个不同的 id（`(节点, 路径)` 是两个项目）。
pub fn project_id_for_on(node_id: &str, canonical_path: &str) -> String {
    use sha2::{Digest, Sha256};
    let material = if node_id == LOCAL_NODE_ID {
        canonical_path.to_string()
    } else {
        // 节点标识不含 `:`（见 validate_node_id），因此第一个 `:` 就是分隔点。
        format!("{node_id}:{canonical_path}")
    };
    let digest = Sha256::digest(material.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("proj-{}", &hex[..12])
}

/// 会话所属项目的稳定 id（add-remote-execution-node 8.1）。
///
/// 节点从 `SessionInfo.remote` 取（`None` = 主控本机会话，用 [`LOCAL_NODE_ID`]）。
/// 少了这一步，远端会话的 `project_dir` 会按本机公式算出「本机同路径项目」的 id，
/// 于是两个机器上的同名路径被并成一个分组——恰恰是 `(节点, 路径)` 身份要防的。
pub fn project_id_for_session(info: &sebas_dispatch::SessionInfo) -> Option<String> {
    let path = info.project_dir.as_deref()?;
    let node = info
        .remote
        .as_ref()
        .map(|r| r.node_id.as_str())
        .unwrap_or(LOCAL_NODE_ID);
    Some(project_id_for_on(node, path))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RegistryFile {
    projects: Vec<ProjectEntry>,
}

/// 节点侧路径判定结果，与 `SessionOp::CheckPath` 的应答同形
/// （`exists` / `is_dir`）。定义在这里而不是复用 `session_backend::PathCheck`，
/// 是为了让 `projects` 模块（纯注册表逻辑）不依赖驱动缝。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodePathCheck {
    pub exists: bool,
    pub is_dir: bool,
}

/// 把**节点**给出的路径判定翻成注册拒绝（add-remote-execution-node 8.1）。
///
/// spec「path does not exist」要求拒绝文案**同时点名节点、路径和哪里不对**；
/// 只说「路径不可用」会让操作者无从判断是哪台机器上的哪条路径。
pub fn validate_remote_path(node_id: &str, path: &str, check: NodePathCheck) -> Result<(), String> {
    if !check.exists {
        return Err(format!("节点 {node_id} 上路径不存在: {path}"));
    }
    if !check.is_dir {
        return Err(format!("节点 {node_id} 上路径不是目录: {path}"));
    }
    Ok(())
}

/// 节点侧校验**没能完成**时的如实拒绝（节点离线 / 链路未接入）。
///
/// 这是一个独立分支而不是「校验失败」：路径可能完全可用，只是这台机器无法
/// 回答。两者混为一谈会把「校验不了」说成「路径不对」。
pub fn node_check_unavailable(node_id: &str, path: &str, cause: &str) -> String {
    format!("无法在节点 {node_id} 上校验路径 {path}: {cause}")
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
            tracing::warn!(path = %path.display(), error = %e, "failed to read projects.json");
            return Vec::new();
        }
    };
    match serde_json::from_str::<RegistryFile>(&raw) {
        Ok(file) => file.projects,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to parse projects.json, returning empty list");
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
    // Backfill for entries persisted by an older registry: `id` (older files
    // predate the stable project id) and `branch_at` (no probe timestamps).
    let mut dirty = false;
    for p in &mut projects {
        if p.id.is_empty() {
            p.id = project_id_for(&p.path);
            dirty = true;
        }
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
    add_on(LOCAL_NODE_ID, path)
}

/// 在指定节点上注册一个项目（add-remote-execution-node 3.2）。
///
/// - **本机**：照旧校验并规范化路径（路径必须真实存在）。
/// - **远端**：只登记 `(节点, 路径)`，**不做任何本地文件系统操作**——那台机器上的
///   路径在这台机器上无从判断，可用性由节点在 spawn 时判定（3.3）。伪造一次本地
///   stat 会把「路径在主控上恰好同名」误当成「路径存在于节点上」。
pub fn add_on(node_id: &str, path: &str) -> Result<ProjectEntry, String> {
    let node_id = node_id.trim();
    if node_id.is_empty() {
        return Err("节点标识不能为空".into());
    }
    let is_local = node_id == LOCAL_NODE_ID;

    let (stored_path, name) = if is_local {
        let dir = Path::new(path);
        if !dir.exists() {
            return Err(format!("路径不存在: {path}"));
        }
        if !dir.is_dir() {
            return Err(format!("路径不是目录: {path}"));
        }
        let canonical_str =
            crate::fs::canonicalize_plain(dir).map_err(|_| format!("无法解析路径: {path}"))?;
        let name = Path::new(&canonical_str)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string());
        (canonical_str, name)
    } else {
        if path.trim().is_empty() {
            return Err("远端项目的路径不能为空".into());
        }
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string());
        (path.to_string(), name)
    };

    let mut projects = load();
    // 重复判定按 `(节点, 路径)`：同一路径在另一台机器上是另一个项目。
    if projects
        .iter()
        .any(|p| p.node_id == node_id && p.path == stored_path)
    {
        return Err(format!("项目已注册: {node_id}:{stored_path}"));
    }

    let added_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let entry = ProjectEntry {
        id: project_id_for_on(node_id, &stored_path),
        path: stored_path,
        name,
        added_at,
        default_agent: None,
        branch: None,
        branch_at: 0,
        node_id: node_id.to_string(),
    };
    projects.push(entry.clone());
    save(&projects)?;
    Ok(entry)
}

/// Remove a project by its stable id. Returns `Ok(true)` if removed,
/// `Ok(false)` if the id is unknown.
pub fn remove_by_id(id: &str) -> Result<bool, String> {
    let mut projects = load();
    let before = projects.len();
    projects.retain(|p| p.id != id);
    if projects.len() == before {
        return Ok(false);
    }
    save(&projects)?;
    Ok(true)
}

/// Record the agent most recently used under a project (project-level
/// default agent, D5). Best-effort: an unknown id or write failure is not a
/// session-creation failure.
pub fn set_default_agent(id: &str, agent: &str) {
    let mut projects = load();
    if let Some(p) = projects.iter_mut().find(|p| p.id == id) {
        p.default_agent = Some(agent.to_string());
        let _ = save(&projects);
    }
}

/// Reorder the registry to match the provided sequence of canonical paths.
/// Paths not in the sequence keep their relative position; unknown paths are
/// appended at the end. Returns the new ordering.
///
/// `(节点, 路径)` 身份（add-remote-execution-node 3.2）之后，key 不能只是 path：
/// 同一路径在两台机器上是两条目，按 path 建 map 会静默丢掉一条。这里按
/// **条目本身**取走（id 优先，其次 path 精确匹配），两条同路径条目都能留下。
pub fn reorder(ordered_paths: &[String]) -> Result<Vec<ProjectEntry>, String> {
    let projects = load();
    let mut remaining: Vec<ProjectEntry> = projects;
    let mut next: Vec<ProjectEntry> = Vec::with_capacity(remaining.len());
    for key in ordered_paths {
        let found = remaining
            .iter()
            .position(|p| p.id == *key || p.path == *key);
        if let Some(pos) = found {
            next.push(remaining.remove(pos));
        }
    }
    // Append any unregistered projects (e.g. manually edited file) at the end.
    let mut tail = remaining;
    tail.sort_by_key(|p| p.added_at);
    next.extend(tail);
    save(&next)?;
    Ok(next)
}

/// 全量保存给定顺序的项目列表（状态库回退路径用：把 DB 列表形状转回
/// ProjectEntry 后写文件注册表）。
pub fn save_ordered(entries: &[ProjectEntry]) -> Result<(), String> {
    save(entries)
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
        tracing::warn!(path = %path, error = %e, "failed to write branch cache");
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
            tracing::warn!(path = %path, error = %e, "failed to write branch cache");
        }
    }
    branch
}

/// Reads `.git/HEAD`. Handles two cases:
/// - Plain ref: `ref: refs/heads/main` → branch = `main`
/// - Detached HEAD: raw commit sha → branch = None (we don't fabricate names)
pub fn probe_git_branch(repo: &Path) -> Option<String> {
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

/// 测试专用：注册表 env（SEBAS_PROJECTS_PATH）的进程级串行锁句柄。
/// server.rs 的 allowed_roots_tests 与本模块测试共用，防止并发测试在
/// 彼此临界区内改写 env。
#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    tests::test_env_lock()
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

    /// 进程级注册表 env（SEBAS_PROJECTS_PATH）的测试串行锁。server.rs 的
    /// allowed_roots_tests 也走文件注册表降级路径，必须与本模块共用一把
    /// 锁，否则并发测试会在彼此的临界区内改写 env。
    pub(super) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner())
    }

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
    #[test] fn remove_project() { with_test_env(|| { let dir = TEST_DIR.path().join("r"); std::fs::create_dir_all(&dir).unwrap(); let s = dir.to_string_lossy(); let added = add(&s).unwrap(); assert!(remove_by_id(&added.id).unwrap()); assert!(list().is_empty()); }); }
    // ── (节点, 路径) 项目身份（add-remote-execution-node 3.2）──────────────

    #[test]
    fn legacy_entry_without_node_id_is_read_back_as_local() {
        // 迁移就靠 serde default：旧注册表里没有 node_id 的条目自动成为本机项目。
        let legacy = r#"{"projects":[{"id":"proj-abc123","path":"/srv/repo","name":"repo","added_at":1}]}"#;
        let file: RegistryFile = serde_json::from_str(legacy).unwrap();
        let entry = &file.projects[0];
        assert_eq!(entry.node_id, LOCAL_NODE_ID, "旧条目回填为本机节点");
        assert!(entry.is_local());
        assert_eq!(entry.branch, None, "其它缺省字段照旧");
    }

    #[test]
    fn local_project_ids_keep_the_historical_formula() {
        // 已注册的本机项目 id 必须原样不变，否则既有项目会变成孤儿。
        assert_eq!(
            project_id_for_on(LOCAL_NODE_ID, "/srv/repo"),
            project_id_for("/srv/repo")
        );
    }

    #[test]
    fn the_same_path_on_two_nodes_is_two_projects() {
        let a = project_id_for_on("node-1", "/srv/repo");
        let b = project_id_for_on("node-2", "/srv/repo");
        let local = project_id_for_on(LOCAL_NODE_ID, "/srv/repo");
        assert_ne!(a, b, "同一路径在两台机器上是两个项目");
        assert_ne!(a, local);
        assert_ne!(b, local);
        // 确定性：同输入同输出（重启/重建后 id 不变）。
        assert_eq!(a, project_id_for_on("node-1", "/srv/repo"));
    }

    #[test]
    fn remote_registration_never_touches_the_local_filesystem() {
        with_test_env(|| {
            // 本机：路径必须真实存在。
            assert!(add("/definitely/not/here").is_err());
            // 远端：路径在那台机器上，主控不做本地 stat —— 登记成功。
            let entry = add_on("dev-box", "/srv/repo").unwrap();
            assert_eq!(entry.node_id, "dev-box");
            assert!(!entry.is_local());
            assert_eq!(entry.path, "/srv/repo", "远端路径按原样保存（不规范化）");
            assert!(entry.branch.is_none(), "远端不做本地分支探测");
        });
    }

    #[test]
    fn duplicate_detection_is_per_node() {
        with_test_env(|| {
            add_on("node-1", "/srv/repo").unwrap();
            // 同一节点同一路径 → 重复。
            assert!(add_on("node-1", "/srv/repo").is_err());
            // 另一台机器上的同一路径 → 另一个项目。
            let other = add_on("node-2", "/srv/repo").unwrap();
            assert_eq!(other.node_id, "node-2");
            assert_eq!(list().len(), 2);
        });
    }

    #[test]
    fn empty_node_id_is_rejected() {
        with_test_env(|| {
            assert!(add_on("", "/srv/repo").is_err());
            assert!(add_on("   ", "/srv/repo").is_err());
            assert!(add_on("dev-box", "   ").is_err(), "远端路径不能为空");
        });
    }

    // ── 远端路径由节点判定（add-remote-execution-node 8.1）────────────────

    #[test]
    fn remote_path_rejection_names_node_path_and_problem() {
        // 路径不存在：文案必须同时点名节点、路径与「不存在」。
        let missing =
            validate_remote_path("dev-box", "/srv/repo", NodePathCheck { exists: false, is_dir: false })
                .unwrap_err();
        assert!(missing.contains("dev-box"), "未点名节点: {missing}");
        assert!(missing.contains("/srv/repo"), "未点名路径: {missing}");
        assert!(missing.contains("不存在"), "未说明哪里不对: {missing}");

        // 存在但不是目录：与「不存在」是两回事，文案必须区分。
        let not_dir =
            validate_remote_path("dev-box", "/srv/file.txt", NodePathCheck { exists: true, is_dir: false })
                .unwrap_err();
        assert!(not_dir.contains("dev-box"));
        assert!(not_dir.contains("/srv/file.txt"));
        assert!(not_dir.contains("不是目录"));
        assert_ne!(not_dir, missing, "两种失败不能共用同一句文案");

        // 可用路径：通过。
        assert!(
            validate_remote_path("dev-box", "/srv/repo", NodePathCheck { exists: true, is_dir: true })
                .is_ok()
        );
    }

    #[test]
    fn node_check_unavailable_is_not_reported_as_a_bad_path() {
        // 「校验不了」与「路径不对」必须分开：文案点名节点/路径/成因，
        // 且不得声称路径有问题（路径可能完全可用）。
        let cause = node_check_unavailable("dev-box", "/srv/repo", "节点离线");
        assert!(cause.contains("dev-box"));
        assert!(cause.contains("/srv/repo"));
        assert!(cause.contains("节点离线"));
        assert!(cause.contains("无法"), "未如实说明校验未完成: {cause}");
        assert!(!cause.contains("不是目录") && !cause.contains("路径不存在"));
    }

    #[test]
    fn reorder_keeps_the_same_path_on_two_nodes() {
        with_test_env(|| {
            add_on("node-1", "/srv/repo").unwrap();
            add_on("node-2", "/srv/repo").unwrap();
            assert_eq!(list().len(), 2);
            // 重排（旧实现按 path 建 map 会把其中一条静默丢掉）。
            let reordered = reorder(&["/srv/repo".to_string()]).unwrap();
            assert_eq!(reordered.len(), 2, "同路径两节点的两条目都必须留下");
            let mut nodes: Vec<_> = reordered.iter().map(|p| p.node_id.clone()).collect();
            nodes.sort();
            assert_eq!(nodes, vec!["node-1".to_string(), "node-2".to_string()]);
        });
    }

    #[test]
    fn reorder_can_target_one_node_by_id() {
        with_test_env(|| {
            let a = add_on("node-1", "/srv/repo").unwrap();
            add_on("node-2", "/srv/repo").unwrap();
            // 按稳定 id 精确取走 node-2 的那条，node-1 留在尾部。
            let reordered = reorder(&[a.id.clone()]).unwrap();
            assert_eq!(reordered.len(), 2);
            assert_eq!(reordered[0].id, a.id, "id 命中项排在前面");
        });
    }

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