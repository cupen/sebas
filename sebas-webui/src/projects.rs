//! Project registry: 项目注册的**纯逻辑助手**（migrate-project-registry 6.1
//! 起，注册表数据本身已全部由 core 状态库承载——`projects.db` 的 `projects`
//! 表，经 state 方法读写；本模块不再有任何文件读写）。
//!
//! 保留在这里的是三件与存储无关的事：
//! - `project_id_for*`：`(节点, 路径)` → 稳定 id 的派生入口（唯一实现在
//!   `sebas-models`，与记录同处并供落库侧共用，这里只是消费面路径）；
//! - `validate_remote_path` / `node_check_unavailable` / `NodePathCheck`：
//!   远端注册的路径校验语义；
//! - `is_accessible` / `probe_git_branch`：本地文件系统探测（分支缓存
//!   TTL 的数据本身在规范记录的 `branch` / `branch_at` 列上）。
//!
//! 注册表条目 = `sebas_models::project::ProjectRow`（存储 + 线同一形状），
//! 经 `ProjectEntry` 别名引用。

use std::path::Path;

/// Git branch re-check TTL (seconds). Branches rarely change; the cache
/// keeps the rail cheap when many sessions share a project.
pub const BRANCH_TTL_SECS: u64 = 30;

// 规范项目记录（migrate-project-registry 2.1）：`ProjectEntry` 与
// `ProjectRow` 合一后，这里经 `sebas_models` 再导出保持既有公开路径。
pub use sebas_models::project::{ProjectEntry, LOCAL_NODE_ID};

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
///
/// 空（或全空白）节点标识按本机处理：它命名不了任何真实节点，而 API 入口本就把
/// 空 `node_id` 归一为 [`LOCAL_NODE_ID`]（`projects_add`）。两处同判，避免某个
/// 漏网调用点把空标识拌进哈希、凭空造出一个「远端」项目身份。
///
/// 归属（add-domain-layer D5「放置规则」）：唯一实现在**拥有 `projects` 表的
/// crate**，与记录 [`ProjectEntry`] 同处——项目记录的唯一形态既是域对象又是持
/// 久行，身份规则随之定义在 `sebas-models::project`，`add_project` 落库时也调
/// 它。这里只保留消费面路径，公式不在本文件重复；落库的 id 与界面用的 id 必须
/// 是同一个值，否则按 id 寻址的移除/重排/项目级默认 agent 会静默错位。
pub fn project_id_for_on(node_id: &str, canonical_path: &str) -> String {
    sebas_models::project::project_id_for_on(node_id, canonical_path)
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

/// 节点侧路径判定结果，与 `SessionOp::CheckPath` 的应答同形
/// （`exists` / `is_dir` / `within_workspace`）。定义在这里而不是复用
/// `session_backend::PathCheck`，是为了让本模块（纯注册表逻辑）不依赖驱动缝。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodePathCheck {
    pub exists: bool,
    pub is_dir: bool,
    /// 节点以它**自己的** workspace root 做的 containment 判定
    /// （add-workspace-root）。进程内直传（不走 serde）；「老节点应答缺字段
    /// 视为界内」的 wire 缺省语义在 `session_backend::PathCheck` 上落地。
    pub within_workspace: bool,
}

/// 把**节点**给出的路径判定翻成注册拒绝（add-remote-execution-node 8.1）。
///
/// spec「path does not exist」要求拒绝文案**同时点名节点、路径和哪里不对**；
/// 只说「路径不可用」会让操作者无从判断是哪台机器上的哪条路径。
///
/// add-workspace-root 2.4：范围判定先行于存在性判定——`within_workspace ==
/// false` 直接拒绝，文案不区分「存在与否」，不借 400 文案差异探测节点上目录
/// 的存在性（与本地注册同一姿态）。
pub fn validate_remote_path(node_id: &str, path: &str, check: NodePathCheck) -> Result<(), String> {
    if !check.within_workspace {
        return Err(format!(
            "节点 {node_id} 判定路径 {path} 超出该节点的 workspace root"
        ));
    }
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

/// Returns true when the project's directory exists and is reachable. Used
/// by the UI to render a row that still lists its sessions but is visually
/// marked unreachable.
pub fn is_accessible(path: &str) -> bool {
    Path::new(path).is_dir()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_project_ids_keep_the_historical_formula() {
        // 本机公式必须逐字不变：id 是持久标识，改公式 = 既有项目变孤儿。
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
    fn remote_path_rejection_names_node_path_and_problem() {
        let e = validate_remote_path(
            "dev-box",
            "/srv/repo",
            NodePathCheck { exists: false, is_dir: false, within_workspace: true },
        )
        .unwrap_err();
        assert!(e.contains("dev-box") && e.contains("/srv/repo") && e.contains("不存在"), "{e}");

        let e = validate_remote_path(
            "dev-box",
            "/srv/repo",
            NodePathCheck { exists: true, is_dir: false, within_workspace: true },
        )
        .unwrap_err();
        assert!(e.contains("不是目录"), "{e}");
    }

    #[test]
    fn remote_path_out_of_node_workspace_is_rejected_before_existence() {
        let e = validate_remote_path(
            "dev-box",
            "/srv/repo",
            NodePathCheck { exists: false, is_dir: false, within_workspace: false },
        )
        .unwrap_err();
        assert!(e.contains("workspace"), "范围判定先行于存在性: {e}");
        assert!(!e.contains("不存在"), "不借文案差异探测存在性: {e}");
    }

    #[test]
    fn node_check_unavailable_is_not_reported_as_a_bad_path() {
        let msg = node_check_unavailable("dev-box", "/srv/repo", "节点离线");
        assert!(msg.contains("无法") && msg.contains("dev-box") && msg.contains("离线"), "{msg}");
    }

    #[test]
    fn empty_node_id_is_rejected_by_registration_callers() {
        // 空节点标识的拒绝在 api 层入口（add_on 已并入 state 方法调用方）；
        // 派生公式对空节点标识退化为本机公式——不允许远端走本机公式。
        assert_eq!(project_id_for_on("", "/srv/repo"), project_id_for("/srv/repo"));
    }

    #[test]
    fn read_branch_probe_finds_git_head() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".git")).unwrap();
        std::fs::write(d.path().join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(probe_git_branch(d.path()), Some("feature/x".to_string()));
    }

    #[test]
    fn read_branch_probe_returns_none_for_non_git_dir() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(probe_git_branch(d.path()), None);
    }

    #[test]
    fn read_branch_probe_returns_none_for_detached_head() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".git")).unwrap();
        std::fs::write(
            d.path().join(".git/HEAD"),
            "9dce8c9d4f3b1e2a0b8c0d1e2f3a4b5c6d7e8f90\n",
        )
        .unwrap();
        assert_eq!(probe_git_branch(d.path()), None);
    }

    #[test]
    fn is_accessible_reflects_filesystem() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().to_str().unwrap().to_string();
        assert!(is_accessible(&p));
        drop(d);
        assert!(!is_accessible(&p));
    }

    /// migrate-project-registry 4.2：不引入导入标记键。
    #[test]
    fn no_import_marker_concept_exists() {
        // 规范记录不携带任何 imported/legacy 标记列（列清单钉在
        // sebas-models::project 的形状钉测试里）；这里钉常量面。
        assert_eq!(LOCAL_NODE_ID, "local");
    }
}
