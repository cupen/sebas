//! 项目域：注册表条目的 JSON 形状（add-domain-layer 3.5）。
//!
//! `ProjectEntry` 是 webui 项目注册表（`projects.json`）与 HTTP API 上的
//! **线形状**。它与持久行形状 `sebas::sebas_state::repo::ProjectRow`
//! （SQLite，`SchemaColumns` 钉在根 crate）是同一概念的**两个合法形状**：
//! 本 change 不合并（合并交 `migrate-project-registry`，见 design D6），
//! 只在根 crate 里提供双向显式转换 + 两侧序列化形状钉测试。

use serde::{Deserialize, Serialize};

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
    /// 必须随节点不同（webui 侧 `project_id_for_on` 派生）。`#[serde(default)]`
    /// 让旧注册表自动回填为 [`LOCAL_NODE_ID`]——那就是迁移，不需要额外的迁移脚本。
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 形状钉（add-domain-layer 3.5 的域侧一半）：完整条目与旧版最小条目
    /// 的序列化形状——可选字段缺席、node_id 缺省回填都逐字钉住。
    #[test]
    fn project_entry_wire_shapes_are_pinned() {
        let full = ProjectEntry {
            id: "proj-abcdef123456".into(),
            path: "/data/work".into(),
            name: "work".into(),
            added_at: 1_700_000_000,
            default_agent: Some("claude".into()),
            branch: Some("main".into()),
            branch_at: 1_700_000_100,
            node_id: "node-b".into(),
        };
        assert_eq!(
            serde_json::to_value(&full).unwrap(),
            serde_json::json!({
                "id": "proj-abcdef123456",
                "path": "/data/work",
                "name": "work",
                "added_at": 1_700_000_000,
                "default_agent": "claude",
                "branch": "main",
                "branch_at": 1_700_000_100,
                "node_id": "node-b",
            })
        );

        // 旧注册表条目（无 id / 无可选字段）：缺省回填，不报错。
        let legacy: ProjectEntry = serde_json::from_str(
            r#"{"path": "/tmp/p2", "name": "p2", "added_at": 5}"#,
        )
        .unwrap();
        assert_eq!(legacy.id, "");
        assert_eq!(legacy.node_id, LOCAL_NODE_ID);
        assert!(legacy.is_local());
        assert_eq!(
            serde_json::to_value(&legacy).unwrap(),
            serde_json::json!({
                "id": "",
                "path": "/tmp/p2",
                "name": "p2",
                "added_at": 5,
                "branch_at": 0,
                "node_id": "local",
            })
        );
    }

    #[test]
    fn foreign_node_entry_is_not_local() {
        let e = ProjectEntry {
            id: "proj-x".into(),
            path: "/tmp/x".into(),
            name: "x".into(),
            added_at: 0,
            default_agent: None,
            branch: None,
            branch_at: 0,
            node_id: "node-b".into(),
        };
        assert!(!e.is_local());
    }
}
