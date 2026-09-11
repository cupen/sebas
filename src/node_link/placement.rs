//! 会话身份发行与放置（add-remote-execution-node 3.1 / 3.4）。
//!
//! ## 身份：由控制面发行，按项目命名空间
//!
//! 设计 D3：会话 id 由**控制面**发行（不可伪造、单一发行者），节点以它为主键。
//! 命名空间取项目，于是「不同项目里的同名会话」不可能撞车——id 形状是
//! `<项目命名空间>:<随机后缀>`；无项目的会话落在 `(no-project)` 命名空间。
//!
//! 反过来，**节点绝不自造 id**：它只接受控制面给的 id 并据此建本地日志。这条不变量
//! 是「会话身份只有一个发行者」的全部含义，也是重挂（对账）能按 id 找回来的前提。
//!
//! ## 放置：是项目的函数，不是调度问题
//!
//! 项目条目是 `(节点, 路径)` 的具名引用，**选项目即选节点**；无项目的会话落到显式
//! 配置的**默认执行节点**（若没配就如实失败——而不是随便挑一台）。节点离线时**不建
//! 占位会话**、也不排队：如实回带着节点名的失败，让操作者立刻知道该去开谁。

use crate::node_link::client::{NodeConnection, NodeLinkError};
use crate::node_link::server::NodeLinkServer;
use sebas_node_link::{SessionOp, SessionResult, SessionRejectCode};

/// 无项目会话的命名空间标签。
pub const NO_PROJECT_NAMESPACE: &str = "(no-project)";

/// 控制面发行的远程会话 id（`<命名空间>:<后缀>`）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteSessionId(String);

impl RemoteSessionId {
    /// 在给定项目命名空间里发行一个新 id（后缀来自操作系统 CSPRNG）。
    pub fn issue(project_id: Option<&str>) -> Self {
        let mut buf = [0u8; 8];
        getrandom::fill(&mut buf).expect("OS CSPRNG unavailable");
        Self::compose(project_id, &hex::encode(buf))
    }

    /// 用给定后缀组装（确定性构造：测试、以及按已存 id 重建场景）。
    pub fn compose(project_id: Option<&str>, suffix: &str) -> Self {
        let namespace = project_id.unwrap_or(NO_PROJECT_NAMESPACE);
        Self(format!("{namespace}:{suffix}"))
    }

    /// 完整 id。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 命名空间（项目）。
    pub fn namespace(&self) -> &str {
        self.0.split_once(':').map(|(ns, _)| ns).unwrap_or(&self.0)
    }

    /// 后缀（项目内名字）。
    pub fn suffix(&self) -> &str {
        self.0.split_once(':').map(|(_, s)| s).unwrap_or("")
    }
}

impl std::fmt::Display for RemoteSessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 项目引用的放置面（`(节点, 路径)`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRef {
    /// 项目标识（同时用作会话 id 的命名空间）。
    pub id: String,
    /// 项目所在节点。
    pub node_id: String,
    /// 项目在**该节点上**的目录路径。
    pub path: String,
}

/// 放在哪台节点上（决策结果，尚未动作）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// 目标节点。
    pub node_id: String,
    /// 项目目录（无项目会话为 `None`，由节点用它的默认工作目录）。
    pub project_dir: Option<String>,
    /// 本项目命名空间下发行的会话 id。
    pub session_id: RemoteSessionId,
}

/// 放置失败。**每一种都指名到底是谁的问题**，便于操作者直接动作。
#[derive(Debug, thiserror::Error)]
pub enum PlacementError {
    /// 无项目会话，且没有配置默认执行节点。
    #[error("该会话没有项目，且未配置默认执行节点：无法决定放在哪台节点上")]
    NoProjectAndNoDefaultNode,
    /// 目标节点当前离线。
    #[error("节点 {node_id} 当前离线：会话未建立（不排队、不建占位）")]
    NodeOffline {
        /// 节点标识。
        node_id: String,
    },
    /// 节点如实拒绝了建立请求。
    #[error("节点 {node_id} 拒绝建立会话（{}）：{cause}", code.as_str())]
    Rejected {
        /// 节点标识。
        node_id: String,
        /// 拒绝码（决定重试与否）。
        code: SessionRejectCode,
        /// 成因。
        cause: String,
    },
    /// 链路层失败（断开 / 超时）。
    #[error("与节点 {node_id} 的链路失败：{source}")]
    Link {
        /// 节点标识。
        node_id: String,
        /// 底层链路错误。
        source: NodeLinkError,
    },
}

/// 建立会话后的实际生效值（连同放置决策一起返回，便于上层如实呈现）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// 放置决策。
    pub placement: Placement,
    /// 节点上报的日志纪元。
    pub epoch: u64,
    /// 实际生效的执行体 kind。
    pub agent_kind: String,
    /// 实际生效的模型。
    pub model: Option<String>,
    /// 实际生效的 mode（节点无法强制时如实回报）。
    pub mode: Option<String>,
    /// 本会话钉住的操作者级材料版本（未使用 → `None`）。
    pub materials_version: Option<String>,
}

/// 解析放置：项目决定节点；无项目落到默认执行节点。
///
/// 只做**决策**，不碰链路——因此可以在没有节点在线时独立测试与呈现。
pub fn resolve(
    project: Option<&ProjectRef>,
    default_node: Option<&str>,
) -> Result<(String, Option<String>, RemoteSessionId), PlacementError> {
    match project {
        Some(project) => Ok((
            project.node_id.clone(),
            Some(project.path.clone()),
            RemoteSessionId::issue(Some(&project.id)),
        )),
        None => {
            let node_id = default_node
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or(PlacementError::NoProjectAndNoDefaultNode)?
                .to_string();
            Ok((node_id, None, RemoteSessionId::issue(None)))
        }
    }
}

/// 决策 + 落地：在目标节点上建立会话。
///
/// 节点不在线时**不建立任何东西**（不排队、不建占位）——占位会话会让操作者以为工作
/// 已经开始，而那正是本设计反复拒绝的「伪装受理」。
pub async fn place_and_spawn(
    server: &NodeLinkServer,
    project: Option<&ProjectRef>,
    default_node: Option<&str>,
    agent_kind: Option<&str>,
    model: Option<&str>,
    mode: Option<&str>,
) -> Result<Placed, PlacementError> {
    let (node_id, project_dir, session_id) = resolve(project, default_node)?;
    let placement = Placement {
        node_id: node_id.clone(),
        project_dir: project_dir.clone(),
        session_id,
    };

    let connection = server
        .live_connection(&node_id)
        .await
        .ok_or_else(|| PlacementError::NodeOffline {
            node_id: node_id.clone(),
        })?;

    let result = connection
        .request(SessionOp::Spawn {
            session_id: placement.session_id.as_str().to_string(),
            project_dir: placement.project_dir.clone(),
            agent_kind: agent_kind.map(str::to_string),
            model: model.map(str::to_string),
            mode: mode.map(str::to_string),
            // 7.1 的 provider 选择由节点侧执行体落地；放置层暂时不下发期望 provider
            // （`None` = 用节点配置的默认），实际生效值由节点在 `Spawned` 里回报。
            provider: None,
        })
        .await
        .map_err(|source| PlacementError::Link {
            node_id: node_id.clone(),
            source,
        })?;

    match result {
        // `..`：节点回报的 provider 等新增字段由 7.1 的落地方接进来（Placed 也随之
        // 扩展）；这里用 `..` 保证协议加字段不会把放置层打挂。
        SessionResult::Spawned {
            epoch,
            agent_kind,
            model,
            mode,
            materials_version,
            ..
        } => Ok(Placed {
            placement,
            epoch,
            agent_kind,
            model,
            mode,
            materials_version,
        }),
        SessionResult::Rejected { code, cause } => Err(PlacementError::Rejected {
            node_id,
            code,
            cause,
        }),
        other => Err(PlacementError::Link {
            node_id,
            source: NodeLinkError::Transport {
                cause: format!("建立会话得到非预期应答：{other:?}"),
            },
        }),
    }
}

/// 在给定连接上建立会话（当调用方已经持有连接句柄时用；避免二次查表）。
pub async fn spawn_on(
    connection: &NodeConnection,
    session_id: RemoteSessionId,
    project_dir: Option<&str>,
    agent_kind: Option<&str>,
    model: Option<&str>,
    mode: Option<&str>,
) -> Result<Placed, PlacementError> {
    let node_id = connection.node_id().to_string();
    let placement = Placement {
        node_id: node_id.clone(),
        project_dir: project_dir.map(str::to_string),
        session_id,
    };
    let result = connection
        .request(SessionOp::Spawn {
            session_id: placement.session_id.as_str().to_string(),
            project_dir: placement.project_dir.clone(),
            agent_kind: agent_kind.map(str::to_string),
            model: model.map(str::to_string),
            mode: mode.map(str::to_string),
            // 7.1 的 provider 选择由节点侧执行体落地；放置层暂时不下发期望 provider
            // （`None` = 用节点配置的默认），实际生效值由节点在 `Spawned` 里回报。
            provider: None,
        })
        .await
        .map_err(|source| PlacementError::Link {
            node_id: node_id.clone(),
            source,
        })?;
    match result {
        // `..`：节点回报的 provider 等新增字段由 7.1 的落地方接进来（Placed 也随之
        // 扩展）；这里用 `..` 保证协议加字段不会把放置层打挂。
        SessionResult::Spawned {
            epoch,
            agent_kind,
            model,
            mode,
            materials_version,
            ..
        } => Ok(Placed {
            placement,
            epoch,
            agent_kind,
            model,
            mode,
            materials_version,
        }),
        SessionResult::Rejected { code, cause } => Err(PlacementError::Rejected {
            node_id,
            code,
            cause,
        }),
        other => Err(PlacementError::Link {
            node_id,
            source: NodeLinkError::Transport {
                cause: format!("建立会话得到非预期应答：{other:?}"),
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_carry_the_project_namespace() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = RemoteSessionId::issue(Some("proj-a"));
            assert!(seen.insert(id.as_str().to_string()), "id 必须唯一：{id}");
            assert_eq!(id.namespace(), "proj-a");
            assert_eq!(id.suffix().len(), 16, "后缀为 8 字节 hex");
        }
    }

    #[test]
    fn the_same_local_name_in_two_projects_does_not_collide() {
        // 「不同项目可同名会话」：即便项目内名字相同，完整 id 也不同。
        let a = RemoteSessionId::compose(Some("proj-a"), "deadbeef");
        let b = RemoteSessionId::compose(Some("proj-b"), "deadbeef");
        assert_ne!(a, b);
        assert_eq!(a.suffix(), b.suffix());
        assert_ne!(a.namespace(), b.namespace());
        assert_eq!(a.as_str(), "proj-a:deadbeef");
    }

    #[test]
    fn project_less_sessions_use_their_own_namespace() {
        let id = RemoteSessionId::issue(None);
        assert_eq!(id.namespace(), NO_PROJECT_NAMESPACE);
        assert!(id.as_str().starts_with("(no-project):"));
    }

    #[test]
    fn project_decides_the_node_and_the_path() {
        let project = ProjectRef {
            id: "proj-a".into(),
            node_id: "dev-box".into(),
            path: "/srv/repo".into(),
        };
        let (node, dir, id) = resolve(Some(&project), Some("other-node")).unwrap();
        assert_eq!(node, "dev-box", "项目决定节点，默认节点不参与");
        assert_eq!(dir.as_deref(), Some("/srv/repo"));
        assert_eq!(id.namespace(), "proj-a");
    }

    #[test]
    fn project_less_sessions_fall_to_the_default_node() {
        let (node, dir, id) = resolve(None, Some("default-node")).unwrap();
        assert_eq!(node, "default-node");
        assert_eq!(dir, None);
        assert_eq!(id.namespace(), NO_PROJECT_NAMESPACE);
    }

    #[test]
    fn project_less_without_a_default_node_fails_honestly() {
        for default in [None, Some(""), Some("   ")] {
            assert!(matches!(
                resolve(None, default),
                Err(PlacementError::NoProjectAndNoDefaultNode)
            ));
        }
    }

    #[test]
    fn the_same_path_on_two_nodes_is_two_placements() {
        let a = ProjectRef {
            id: "proj-a".into(),
            node_id: "node-1".into(),
            path: "/srv/repo".into(),
        };
        let b = ProjectRef {
            id: "proj-b".into(),
            node_id: "node-2".into(),
            path: "/srv/repo".into(),
        };
        let (node_a, dir_a, id_a) = resolve(Some(&a), None).unwrap();
        let (node_b, dir_b, id_b) = resolve(Some(&b), None).unwrap();
        assert_ne!(node_a, node_b);
        assert_eq!(dir_a, dir_b, "路径相同…");
        assert_ne!(id_a.namespace(), id_b.namespace(), "…但是两个不同的项目");
    }
}
