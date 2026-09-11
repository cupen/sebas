//! 远端会话的生命周期与**节点绑定**（add-remote-execution-node 5.6）。
//!
//! 三种情形必须给出**三种不同**的结论，这是本模块存在的全部理由：
//!
//! | 发生了什么 | 会话怎么了 | 依据 |
//! |---|---|---|
//! | 链路断了 | **没终止**，只是暂时看不见（`NodeOffline`） | rung ③⁺：主控缺席不终止远端执行 |
//! | 主控自己重启了 | **没终止**，重建视图后照样在（`Live`） | 执行事实在节点上，主控的副本可重建 |
//! | 节点重启了 | **终止**，且成因来自节点（`Terminated`） | 节点是子进程寿命的持有者 |
//!
//! 另外两条硬规矩：
//!
//! - **对账永不重建**：本模块没有任何路径会发出 `Spawn`。节点上没了的会话就是没了
//!   （截图、终止、如实报告），不会"顺手建一个一样的"。
//! - **不丢事实**：节点重启后日志仍在节点磁盘上（宿主会把孤立日志挂回来），因此
//!   终止之后**仍可回拉尾部条目**——控制面缺的那几轮不会因为重启而永久消失。

use crate::node_link::client::{NodeConnection, NodeLinkError};
use crate::node_link::driver::RemoteSession;
use sebas_node_link::{SessionOp, SessionResult};
use std::collections::HashMap;

/// 一个被跟踪会话的生命周期。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLifecycle {
    /// 节点确认持有（相位由节点给出）。
    Live {
        /// 节点上报的相位。
        phase: String,
    },
    /// 链路断了：**没有终止**，只是暂时联系不上（等重连或对账）。
    NodeOffline {
        /// 节点标识。
        node_id: String,
    },
    /// 已终止：成因来自节点，或是「节点侧已不存在」。
    Terminated {
        /// 成因。
        cause: String,
    },
}

impl SessionLifecycle {
    /// 是否仍算「活着」（在线或暂时离线都算）。
    pub fn is_alive(&self) -> bool {
        !matches!(self, SessionLifecycle::Terminated { .. })
    }
}

/// 对账结果（每一步都可解释，便于上层如实呈现而不是笼统地"同步完成"）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// 节点确认仍持有、已恢复跟踪的会话。
    pub resumed: Vec<String>,
    /// 判定为终止的会话及其成因。
    pub terminated: Vec<(String, String)>,
    /// 本次对账后仍悬空的审批请求总数。
    pub parked_approvals: usize,
}

struct Tracked {
    node_id: String,
    view: RemoteSession,
    lifecycle: SessionLifecycle,
}

/// 控制面侧的远端会话舰队视图。
#[derive(Default)]
pub struct RemoteFleet {
    sessions: HashMap<String, Tracked>,
}

impl RemoteFleet {
    /// 空视图。
    pub fn new() -> Self {
        Self::default()
    }

    /// 开始跟踪一个会话（控制面刚在节点上建立它，或从自己的持久记录里重建）。
    pub fn track(&mut self, node_id: &str, session_id: &str) -> &mut RemoteSession {
        let entry = self
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| Tracked {
                node_id: node_id.to_string(),
                view: RemoteSession::new(session_id),
                lifecycle: SessionLifecycle::Live {
                    phase: "spawning".into(),
                },
            });
        entry.node_id = node_id.to_string();
        &mut entry.view
    }

    /// 会话所在节点（未跟踪 → `None`）。
    pub fn node_of(&self, session_id: &str) -> Option<&str> {
        self.sessions.get(session_id).map(|t| t.node_id.as_str())
    }

    /// 生命周期（未跟踪 → `None`）。
    pub fn lifecycle(&self, session_id: &str) -> Option<&SessionLifecycle> {
        self.sessions.get(session_id).map(|t| &t.lifecycle)
    }

    /// 本地副本（未跟踪 → `None`）。
    pub fn view(&self, session_id: &str) -> Option<&RemoteSession> {
        self.sessions.get(session_id).map(|t| &t.view)
    }

    /// 本地副本（可变）。
    pub fn view_mut(&mut self, session_id: &str) -> Option<&mut RemoteSession> {
        self.sessions.get_mut(session_id).map(|t| &mut t.view)
    }

    /// 标记某会话仍在节点上活着（快照/对账确认）。
    ///
    /// 与 [`Self::set_terminated`] 配对：投影层按节点的事实整体校正生命周期，
    /// 而不是"遇到什么改什么"——否则一次漏报就会把活着的会话永远留在终止态。
    pub fn set_live(&mut self, session_id: &str, phase: impl Into<String>) {
        if let Some(tracked) = self.sessions.get_mut(session_id) {
            tracked.lifecycle = SessionLifecycle::Live {
                phase: phase.into(),
            };
        }
    }

    /// 标记某会话已终止（成因来自节点，或「节点侧已不存在」）。
    pub fn set_terminated(&mut self, session_id: &str, cause: impl Into<String>) {
        if let Some(tracked) = self.sessions.get_mut(session_id) {
            tracked.lifecycle = SessionLifecycle::Terminated {
                cause: cause.into(),
            };
        }
    }

    /// 某节点的链路回来了：把它那些**只是离线**的会话恢复成在跟踪状态。
    ///
    /// 只动 `NodeOffline`，**不动 `Terminated`**：终止是终局，节点没有"复活"这个
    /// 语义（真的又出现了会被随后的对账按节点的事实改正）。相位此时还不知道，
    /// 写 `"unknown"`——等对账来填，而不是拿链路在线冒充会话在跑。
    pub fn set_node_live(&mut self, node_id: &str) -> usize {
        let mut flipped = 0;
        for tracked in self.sessions.values_mut() {
            let offline = matches!(
                &tracked.lifecycle,
                SessionLifecycle::NodeOffline { node_id: n } if n == node_id
            );
            if offline {
                tracked.lifecycle = SessionLifecycle::Live {
                    phase: "unknown".into(),
                };
                flipped += 1;
            }
        }
        flipped
    }

    /// 全部被跟踪的会话 id（升序）。
    ///
    /// 投影层要按会话逐条产出 UI 行（5.1），所以需要一个确定的遍历顺序；排序也
    /// 让「快照两次结果相同」这种幂等断言可以直接比较。
    pub fn session_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.sessions.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// 某节点上的全部会话。
    pub fn sessions_on(&self, node_id: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, t)| t.node_id == node_id)
            .map(|(id, _)| id.clone())
            .collect();
        out.sort();
        out
    }

    /// 链路断了：该节点的会话转为**暂时离线**——**不终止**。
    ///
    /// 它们的子进程还在那台机器上跑；主控只是看不见。谁都不许在这一步杀会话。
    pub fn on_node_disconnected(&mut self, node_id: &str) -> usize {
        let mut affected = 0;
        for tracked in self.sessions.values_mut() {
            if tracked.node_id == node_id
                && matches!(tracked.lifecycle, SessionLifecycle::Live { .. })
            {
                tracked.lifecycle = SessionLifecycle::NodeOffline {
                    node_id: node_id.to_string(),
                };
                affected += 1;
            }
        }
        affected
    }

    /// 与某节点对账**已跟踪**的会话：节点有的恢复跟踪（并拉日志/审批），
    /// 节点没有的判定终止。**任何情况下都不会建立会话**。
    pub async fn reconcile_node(
        &mut self,
        connection: &NodeConnection,
    ) -> Result<ReconcileReport, NodeLinkError> {
        let node_id = connection.node_id().to_string();
        let on_node = node_sessions(connection).await?;
        let tracked_on_node = self.sessions_on(&node_id);

        let mut report = ReconcileReport::default();
        for session_id in tracked_on_node {
            match on_node.iter().find(|s| s.session_id == session_id) {
                Some(summary) => {
                    if let Some(view) = self.view_mut(&session_id) {
                        // 日志与审批都按节点为准；两个调用都是幂等的。
                        let _ = view.reconcile(connection).await;
                        let parked = view.reconcile_approvals(connection).await.unwrap_or(0);
                        report.parked_approvals += parked;
                        view.note_phase(summary.phase.clone());
                    }
                    let lifecycle = if is_terminal_phase(&summary.phase) {
                        SessionLifecycle::Terminated {
                            cause: format!("节点上报相位 {}", summary.phase),
                        }
                    } else {
                        SessionLifecycle::Live {
                            phase: summary.phase.clone(),
                        }
                    };
                    let terminated = matches!(lifecycle, SessionLifecycle::Terminated { .. });
                    if let Some(tracked) = self.sessions.get_mut(&session_id) {
                        tracked.lifecycle = lifecycle;
                    }
                    if terminated {
                        report.terminated.push((
                            session_id,
                            format!("节点上报相位 {}", summary.phase),
                        ));
                    } else {
                        report.resumed.push(session_id);
                    }
                }
                None => {
                    let cause = format!("节点 {node_id} 侧已不存在（节点重启或会话已被回收）");
                    if let Some(tracked) = self.sessions.get_mut(&session_id) {
                        tracked.lifecycle = SessionLifecycle::Terminated {
                            cause: cause.clone(),
                        };
                    }
                    report.terminated.push((session_id, cause));
                }
            }
        }
        Ok(report)
    }

    /// 从节点**认领**本视图还不认识的会话（控制面自己的记录丢了时的恢复路径）。
    ///
    /// 认领不是发行：这些 id 本来就是控制面发行的（带项目命名空间），这里只是把
    /// 身份与日志取回来。**不建立任何东西**。
    pub async fn adopt_from_node(
        &mut self,
        connection: &NodeConnection,
    ) -> Result<Vec<String>, NodeLinkError> {
        let node_id = connection.node_id().to_string();
        let on_node = node_sessions(connection).await?;
        let mut adopted = Vec::new();
        for summary in on_node {
            if self.sessions.contains_key(&summary.session_id) {
                continue;
            }
            let session_id = summary.session_id.clone();
            let view = RemoteSession::new(session_id.clone());
            let lifecycle = if is_terminal_phase(&summary.phase) {
                SessionLifecycle::Terminated {
                    cause: format!("节点上报相位 {}", summary.phase),
                }
            } else {
                SessionLifecycle::Live {
                    phase: summary.phase.clone(),
                }
            };
            self.sessions.insert(
                session_id.clone(),
                Tracked {
                    node_id: node_id.clone(),
                    view,
                    lifecycle,
                },
            );
            adopted.push(session_id);
        }
        Ok(adopted)
    }
}

/// 节点上报的相位是否意味着「已经结束」。
fn is_terminal_phase(phase: &str) -> bool {
    matches!(phase, "terminated" | "closed" | "exited" | "failed")
}

/// 问节点它现在持有哪些会话。
async fn node_sessions(
    connection: &NodeConnection,
) -> Result<Vec<sebas_node_link::SessionSummary>, NodeLinkError> {
    match connection.request(SessionOp::ListSessions).await? {
        SessionResult::Sessions { sessions } => Ok(sessions),
        SessionResult::Rejected { code, cause } => Err(NodeLinkError::Transport {
            cause: format!("列出会话被拒绝（{}）：{cause}", code.as_str()),
        }),
        other => Err(NodeLinkError::Transport {
            cause: format!("列出会话得到非预期应答：{other:?}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_loss_is_not_a_termination() {
        let mut fleet = RemoteFleet::new();
        fleet.track("dev-box", "proj-a:1");
        fleet.track("dev-box", "proj-a:2");
        fleet.track("other-node", "proj-b:1");

        assert_eq!(fleet.on_node_disconnected("dev-box"), 2, "只影响该节点的会话");
        assert_eq!(
            fleet.lifecycle("proj-a:1"),
            Some(&SessionLifecycle::NodeOffline {
                node_id: "dev-box".into()
            })
        );
        assert!(fleet.lifecycle("proj-a:1").unwrap().is_alive(), "离线≠终止");
        assert!(
            matches!(
                fleet.lifecycle("proj-b:1"),
                Some(SessionLifecycle::Live { .. })
            ),
            "别的节点不受影响"
        );
    }

    #[test]
    fn tracking_is_idempotent_and_rebuildable() {
        let mut fleet = RemoteFleet::new();
        fleet.track("dev-box", "proj-a:1");
        fleet.track("dev-box", "proj-a:1");
        assert_eq!(fleet.sessions_on("dev-box").len(), 1, "重复跟踪不产生两条");
        // 重建（控制面重启）：新视图 + 重新跟踪 + 对账 = 恢复。
        let rebuilt = RemoteFleet::new();
        assert!(rebuilt.view("proj-a:1").is_none());
        let mut rebuilt = rebuilt;
        rebuilt.track("dev-box", "proj-a:1");
        assert_eq!(rebuilt.sessions_on("dev-box"), vec!["proj-a:1".to_string()]);
    }

    #[test]
    fn terminal_phases_are_recognised() {
        for phase in ["terminated", "closed", "exited", "failed"] {
            assert!(is_terminal_phase(phase), "{phase}");
        }
        for phase in ["active", "spawning", "waiting_approval", "idle"] {
            assert!(!is_terminal_phase(phase), "{phase}");
        }
    }

    #[test]
    fn unknown_sessions_have_no_lifecycle() {
        let fleet = RemoteFleet::new();
        assert!(fleet.lifecycle("nope").is_none());
        assert!(fleet.node_of("nope").is_none());
        assert!(fleet.view("nope").is_none());
    }
}
