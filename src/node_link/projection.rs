//! core 侧**远端会话投影**（add-remote-execution-node 5.1 / 5.3）。
//!
//! 事实在节点上，这里是一份**可重建的副本**：控制面（以及它背后的工作台）要能
//! 看见「哪台机器上跑着哪些会话、它们按什么 mode 在跑、是不是在等人批、那台机器
//! 现在还通不通」。这三件事都不该由主控猜——所以本模块只做**投影**，不做裁决：
//! 所有状态都来自节点的快照与事件流，节点没说的就是不写。
//!
//! ## 两条路径，一段代码（5.3）
//!
//! 「链路抖动后重连」与「主控进程重启后重建」在投影上是**同一件事**：视图空了/
//! 旧了，于是按节点的事实重新对账。因此 [`RemoteProjection::observe_node`] 是唯一
//! 的重建入口，两条路径都只调它；内部逐会话调 [`RemoteSession::reconcile`]（从游标
//! +1 回拉，幂等——重复应用同一段结果不变）。
//!
//! ## 为什么订阅在"接入"时就建立
//!
//! 事件消费者的生命周期绑在**连接**上（[`RemoteProjection::attach_connection`]），
//! 而不是绑在对账（`observe_node`）上：对账之前节点就可能已经在产出（刚建好的会话
//! 立刻跑了一轮），只在对账里起消费者会漏掉那一段，而漏掉的事件不会自己回来。
//! 先订阅、后对账也因此是同一个理由的另一面——重复应用是幂等的（`note_batch` 按
//! seq 去重），所以既不会漏也不会重复。
//!
//! ## 三种状态不许混为一谈
//!
//! 链路断了（`offline`，**没终止**）、节点说它结束了（`terminated`，成因来自节点）、
//! 节点侧压根没有这个会话（也是 `terminated`，但成因写「节点侧已不存在」）——展示层
//! 需要这三种结论分别可呈现，因此 `remote.node_status` 与 `remote.node_cause` 一起给出。

use crate::node_link::client::{NodeConnection, NodeLinkError};
use crate::node_link::fleet::{RemoteFleet, SessionLifecycle, ReconcileReport};
use crate::node_link::placement::{PlacementError, RemoteSessionId};
use crate::node_link::server::ConnectionObserver;
use sebas_channels::ChannelKey;
use sebas_dispatch::{RemoteSessionView, SessionEvent, SessionInfo, TurnEntry};
use sebas_webui::session_backend::{PermissionDecision, PermissionNotice};
use sebas_node_link::{
    ApprovalDecision, LogEntry, ParkedApproval, SessionEvent as NodeEvent, SessionOp,
    SessionResult,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};

/// 主控本机的隐式节点标识（与工作台的 `projects::LOCAL_NODE_ID` 是同一个词）。
pub const LOCAL_NODE_ID: &str = "local";

/// 事件广播容量：慢订阅者看到 `Lagged`（由它自己重新快照），不拖住别人。
const EVENT_CAPACITY: usize = 256;

/// 审批广播容量（与本地执行体的审批出口同量级）。
const NOTICE_CAPACITY: usize = 64;

/// 远端会话在控制面侧的稳定行键（`ChannelKey.reference`）。
///
/// 用 `\0` 分隔而不是 `/`：`reference` 里用 NUL 分层是既有的约定（飞书的
/// `chat_id\0thread_id` 就是），而 `decode_session_key` 只按**第一个** NUL 切分，
/// 因此多一层不会破坏解析。键是**确定性**的（节点 id + 会话 id），所以主控重启后
/// 同一个远端会话还是同一行——否则每次重启都会多出一批"新会话"。
pub fn row_reference(node_id: &str, session_id: &str) -> String {
    format!("node\0{node_id}\0{session_id}")
}

/// 拆回（行键 → 节点 id + 会话 id）。不是远端行则 `None`。
pub fn parse_reference(reference: &str) -> Option<(&str, &str)> {
    let rest = reference.strip_prefix("node\0")?;
    rest.split_once('\0')
}

/// 节点上报的相位是否意味着「已经结束」（与 `fleet` 同一判据）。
fn is_terminal_phase(phase: &str) -> bool {
    matches!(phase, "terminated" | "closed" | "exited" | "failed")
}

/// 一个远端会话的**呈现**元数据（节点的事实里没有的那部分）。
///
/// 项目目录、期望/生效 mode、模型与 agent kind 都是从会话创建参数与摘要里拿来的；
/// 拿不到就留 `None`——不猜。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Meta {
    /// 会话来源渠道（工作台起的会话是 `web`）。远端会话的渠道由**控制面**记着，
    /// 节点不知道这件事。
    channel: String,
    project_dir: Option<String>,
    prompt: Option<String>,
    desired_mode: Option<String>,
    effective_mode: Option<String>,
    agent_kind: Option<String>,
    model: Option<String>,
    /// 期望/实际生效的节点 provider profile（7.1）。
    desired_provider: Option<String>,
    provider: Option<String>,
    provider_cause: Option<String>,
    /// 最近一次活动的 unix 秒（取日志条目自带的时间戳，不用本地时钟冒充）。
    last_active_unix: i64,
}

/// core 侧的远端会话投影。
pub struct RemoteProjection {
    /// 会话跟踪（生命周期 / 纪元 / 游标 / 缺口 / 悬空审批）——5.6 已测得比较透。
    fleet: Mutex<RemoteFleet>,
    /// 展示元数据，按会话 id 索引。
    ///
    /// **锁序约定**：先 `fleet` 后 `meta`，全模块一致（反过来会死锁）。
    meta: Mutex<HashMap<String, Meta>>,
    /// 节点不可达的成因（在线则该节点无条目）。链路断开只告知节点 id，成因得自己记。
    node_cause: Mutex<HashMap<String, String>>,
    /// 在线连接（节点标识 → 句柄）。控制面**从这里**把会话操作送到节点上——
    /// 单独存一份是为了让投影既知道"有哪些会话"，也能真的够到它们。
    connections: Mutex<HashMap<String, Arc<NodeConnection>>>,
    /// 会话事件广播：core 的订阅流把它与本地事件合并后推给客户端。
    events: broadcast::Sender<SessionEvent>,
    /// 远端**审批请求**的广播：与本地执行体的审批走同一个出口，工作台因此
    /// 不必区分"这条待批是主控本机的还是别的机器上的"（8.4）。
    notices: broadcast::Sender<PermissionNotice>,
}

impl RemoteProjection {
    /// 空投影。
    pub fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (notices, _) = broadcast::channel(NOTICE_CAPACITY);
        Arc::new(Self {
            fleet: Mutex::new(RemoteFleet::new()),
            meta: Mutex::new(HashMap::new()),
            node_cause: Mutex::new(HashMap::new()),
            connections: Mutex::new(HashMap::new()),
            events,
            notices,
        })
    }

    /// 订阅会话事件（订阅流用）。
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    /// 全部远端会话行（供快照合并）。
    pub async fn rows(&self) -> Vec<SessionInfo> {
        let ids = self.fleet.lock().await.session_ids();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(row) = self.row_of(&id).await {
                out.push(row);
            }
        }
        out
    }

    /// 单个远端会话行（`None` = 这不是一个被跟踪的远端会话）。
    pub async fn row(&self, key: &ChannelKey) -> Option<SessionInfo> {
        let (_, session_id) = parse_reference(&key.reference)?;
        let row = self.row_of(session_id).await?;
        // 渠道必须对上：否则 `web:node\0n\0s` 与 `feishu:node\0n\0s` 会互相冒充。
        (row.channel == key.channel.as_str()).then_some(row)
    }

    /// 这个 key 是不是远端会话？是则给出（节点 id，会话 id）。
    pub async fn remote_target(&self, key: &ChannelKey) -> Option<(String, String)> {
        let (node_id, session_id) = parse_reference(&key.reference)?;
        let fleet = self.fleet.lock().await;
        let tracked_node = fleet.node_of(session_id)?.to_string();
        (tracked_node == node_id).then(|| (node_id.to_string(), session_id.to_string()))
    }

    /// 该会话钉住的操作者级材料版本（未使用材料 → `None`）。
    pub async fn materials_version(&self, session_id: &str) -> Option<String> {
        self.fleet
            .lock()
            .await
            .view(session_id)?
            .materials_version()
            .map(str::to_string)
    }

    /// 该会话当前悬空的审批请求（8.4：返回后要可达）。
    pub async fn parked(&self, session_id: &str) -> Vec<ParkedApproval> {
        self.fleet
            .lock()
            .await
            .view(session_id)
            .map(|v| v.parked_approvals().to_vec())
            .unwrap_or_default()
    }

    /// 该会话的转写条目（位置 = 节点日志的 seq，所以缺口天然可见）。
    ///
    /// 只给**当前纪元**的段：纪元变了就是另一条时间线，把两段接起来等于伪造历史。
    pub async fn turns(&self, session_id: &str, from: u64) -> Vec<TurnEntry> {
        let fleet = self.fleet.lock().await;
        let Some(view) = fleet.view(session_id) else {
            return Vec::new();
        };
        view.entries()
            .iter()
            .filter(|e| e.seq >= from)
            .map(turn_entry_of)
            .collect()
    }

    /// 会话最后一条 seq（UI 判断"还有没有新的"用）。
    pub async fn last_seq(&self, session_id: &str) -> Option<u64> {
        self.fleet.lock().await.view(session_id).map(|v| v.cursor())
    }

    /// 就地登记一个会话（工作台在该节点上新建会话时调用）。
    pub async fn track(&self, node_id: &str, session_id: &str, meta: Meta0) -> ChannelKey {
        let key = ChannelKey::new(meta.channel.clone(), row_reference(node_id, session_id));
        {
            let mut fleet = self.fleet.lock().await;
            // track 是幂等的：同一会话重复登记不会变成两条（节点上只有一个）。
            fleet.track(node_id, session_id);
            let mut metas = self.meta.lock().await;
            metas.insert(session_id.to_string(), meta.into_meta());
        }
        if let Some(row) = self.row_of(session_id).await {
            let _ = self.events.send(SessionEvent::Created { session: row });
        }
        key
    }

    /// 消费一个节点事件并广播更新后的行。返回会话 id（不属于本投影时 `None`）。
    pub async fn apply_node_event(&self, node_id: &str, event: &NodeEvent) -> Option<String> {
        let session_id = event_session_id(event)?;
        let changed = {
            let mut fleet = self.fleet.lock().await;
            // 节点在讲一个本投影还不认识的会话：先认领身份再入账，否则这条事实
            // 会被丢掉（重启后事件可能早于快照到达）。
            if fleet.node_of(session_id).is_none() {
                fleet.track(node_id, session_id);
            }
            let changed = fleet
                .view_mut(session_id)
                .map(|v| v.note_event(event))
                .unwrap_or(false);
            // 生命周期跟着事件流走：能收到事件就说明链路在（离线 → 在线），而
            // 「节点说它结束了」必须记成终止，否则一个已经死掉的会话会一直显示成活着。
            let (dead, phase) = {
                let view = fleet.view(session_id);
                (
                    view.and_then(|v| v.terminated().map(str::to_string)),
                    view.map(|v| v.phase().to_string()).unwrap_or_default(),
                )
            };
            match dead {
                Some(cause) => fleet.set_terminated(session_id, cause),
                None => fleet.set_live(session_id, phase),
            }
            changed
        };
        // 批里的时间戳是真时间；用它推进 last_active，不用本地时钟。
        if let NodeEvent::TurnBatch { entries, .. } = event
            && let Some(last) = entries.last()
            && last.at_unix > 0
        {
            let mut metas = self.meta.lock().await;
            let m = metas.entry(session_id.to_string()).or_default();
            m.last_active_unix = m.last_active_unix.max(last.at_unix);
        }
        // 审批请求要**立刻**上行（不受合并窗口约束）：操作者等的是它，
        // 而且一个在等人批的会话不该看起来像在跑。
        if let NodeEvent::ApprovalRequested {
            session_id,
            request_id,
            tool,
            category,
            ..
        } = event
            && let Some(node_id) = self.fleet.lock().await.node_of(session_id).map(str::to_string)
        {
            let key = ChannelKey::new("web", row_reference(&node_id, session_id));
            let _ = self.notices.send(PermissionNotice {
                request_id: request_id.clone(),
                session_id: encode_session_key(&key),
                tool_name: tool.clone(),
                args: serde_json::json!({ "category": format!("{category:?}") }),
                reason: format!("远端节点 {node_id} 上的受门控动作，等待决定"),
            });
        }
        if changed {
            self.publish_updated(session_id).await;
        }
        Some(session_id.to_string())
    }

    /// 按节点的事实重建/校正视图（5.1 + 5.3 的唯一入口）。
    ///
    /// 幂等：重复调用结果不变（`note_snapshot` 与 `reconcile` 都幂等）。
    pub async fn observe_node(
        self: &Arc<Self>,
        connection: &NodeConnection,
    ) -> Result<ReconcileReport, NodeLinkError> {
        let node_id = connection.node_id().to_string();

        let sessions = list_sessions(connection).await?;
        let mut resumed = Vec::new();
        let mut terminated = Vec::new();
        let known: Vec<String> = {
            let mut fleet = self.fleet.lock().await;
            for summary in &sessions {
                if fleet.node_of(&summary.session_id).is_none() {
                    // 认领：身份本来就是控制面发行的，这里只是把日志取回来，**不重建**。
                    fleet.track(&node_id, &summary.session_id);
                }
                if let Some(view) = fleet.view_mut(&summary.session_id) {
                    // 只校正**状态**，不推进游标：此刻我们手上一条条目都还没有，
                    // 推游标就等于宣称"历史已经在我这儿了"（见 `note_state` 的说明）。
                    // 回收水位线在单会话 `Snapshot` 里才拿得到，此时传 0 不会让已知
                    // 水位线倒退——`note_reclaimed` 取的是 max。
                    view.note_state(summary, 0);
                }
                // 摘要里没有的字段保持原值：认领回来的会话在控制面这边没有创建
                // 上下文（项目目录/提示词），那就留空，不编一个出来。
                let mut metas = self.meta.lock().await;
                let entry = metas.entry(summary.session_id.clone()).or_default();
                if let Some(v) = summary.desired_mode.clone() {
                    entry.desired_mode = Some(v);
                }
                if let Some(v) = summary.mode.clone() {
                    entry.effective_mode = Some(v);
                }
                if let Some(v) = summary.agent_kind.clone() {
                    entry.agent_kind = Some(v);
                }
                if let Some(v) = summary.model.clone() {
                    entry.model = Some(v);
                }
                // provider：期望与实际生效分开记（7.1）。应用不上时 `mode`/`provider`
                // 只有期望值 + 成因，界面据此显示"没生效"，而不是把期望值当结果。
                entry.desired_provider = summary.desired_provider.clone();
                entry.provider = summary.provider.clone();
                entry.provider_cause = summary.provider_cause.clone();
                if entry.channel.is_empty() {
                    entry.channel = "web".into();
                }
            }
            let mut ids = Vec::new();
            for summary in &sessions {
                let terminal = is_terminal_phase(&summary.phase);
                let session_id = summary.session_id.clone();
                if terminal {
                    let cause = format!("节点上报相位 {}", summary.phase);
                    fleet.set_terminated(&session_id, cause.clone());
                    terminated.push((session_id.clone(), cause));
                } else {
                    fleet.set_live(&session_id, summary.phase.clone());
                    resumed.push(session_id.clone());
                }
                ids.push(session_id);
            }
            // 该节点上一个会话都没有了：此前跟踪着的那些是「节点侧已不存在」。
            for session_id in fleet.sessions_on(&node_id) {
                if !ids.contains(&session_id) {
                    let cause = format!("节点 {node_id} 上已不存在该会话");
                    fleet.set_terminated(&session_id, cause.clone());
                    terminated.push((session_id, cause));
                }
            }
            ids
        };

        // 逐会话：**先增量回拉，再拿快照校正**。
        //
        // 顺序不能反，反了会静默丢历史：`note_snapshot` 会把游标推到节点日志的
        // 末尾（`cursor = max(cursor, last_seq)`），于是在一张**重建出来的空视图**
        // 上先应用快照，就等于宣称"我已经有到 last_seq 的全部条目了"——而实际上
        // 一条都没有；随后的 `reconcile` 从 `cursor + 1` 回拉，自然什么都拉不到。
        // 控制面重启后看到的就是一个没有任何转写的会话（进程级 e2e 抓到的正是这个）。
        //
        // 反过来：先按游标回拉（重建时游标为 0，即整段历史），再用快照校正纪元/
        // 相位/材料版本/回收水位线——这些才是快照真正要回答的问题。
        for session_id in &known {
            let pulled = {
                let mut fleet = self.fleet.lock().await;
                match fleet.view_mut(session_id) {
                    Some(view) => view.reconcile(connection).await,
                    None => continue,
                }
            };
            if let Err(e) = pulled {
                // 回拉失败不该让整个对账假装成功：这一条如实记下，其余继续。
                eprintln!("node-link: 会话 {session_id} 对账回拉失败：{e}");
            }

            // 回拉之后再校正：水位线（`ListSessions` 的摘要里没有它）与相位。
            let snapshot = connection
                .request(SessionOp::Snapshot {
                    session_id: session_id.clone(),
                })
                .await;
            {
                let mut fleet = self.fleet.lock().await;
                match (snapshot, fleet.view_mut(session_id)) {
                    (
                        Ok(SessionResult::Snapshot {
                            summary,
                            reclaimed_through_seq,
                            ..
                        }),
                        Some(view),
                    ) => {
                        view.note_snapshot(&summary, reclaimed_through_seq);
                    }
                    (Ok(SessionResult::Rejected { code, cause }), _) => {
                        eprintln!(
                            "node-link: 会话 {session_id} 快照被拒绝（{}）：{cause}",
                            code.as_str()
                        );
                    }
                    (Err(e), _) => {
                        eprintln!("node-link: 会话 {session_id} 快照失败：{e}");
                    }
                    _ => {}
                }
            }
        }

        self.set_node_online(&node_id).await;
        for session_id in &known {
            self.publish_updated(session_id).await;
        }
        for (session_id, _) in &terminated {
            self.publish_updated(session_id).await;
        }

        // 回来的操作者要能看到"谁在等"（8.4）：把仍在悬空的请求主动播一遍。
        self.announce_parked().await;

        let parked_approvals = {
            let fleet = self.fleet.lock().await;
            resumed
                .iter()
                .filter_map(|id| fleet.view(id))
                .map(|v| v.parked_approvals().len())
                .sum()
        };
        Ok(ReconcileReport {
            resumed,
            terminated,
            parked_approvals,
        })
    }

    /// 把某节点的会话全部标成**暂时看不见**（链路断了，不是终止）。
    ///
    /// 返回受影响的会话数。
    pub async fn set_node_offline(&self, node_id: &str, cause: impl Into<String>) -> usize {
        let cause = cause.into();
        self.node_cause
            .lock()
            .await
            .insert(node_id.to_string(), cause);
        let affected = {
            let mut fleet = self.fleet.lock().await;
            fleet.on_node_disconnected(node_id)
        };
        let ids = {
            let fleet = self.fleet.lock().await;
            fleet.sessions_on(node_id)
        };
        for id in &ids {
            self.publish_updated(id).await;
        }
        affected
    }

    /// 节点恢复：清掉离线成因（行内容本身由随后的对账刷新）。
    pub async fn set_node_online(&self, node_id: &str) -> usize {
        self.node_cause.lock().await.remove(node_id);
        {
            let mut fleet = self.fleet.lock().await;
            fleet.set_node_live(node_id);
        }
        let ids = {
            let fleet = self.fleet.lock().await;
            fleet.sessions_on(node_id)
        };
        for id in &ids {
            self.publish_updated(id).await;
        }
        ids.len()
    }

    /// 记下一条在线连接，并**立刻开始消费它的事件流**（观察者在接入时调用）。
    ///
    /// 事件消费者绑在**连接**上而不是绑在对账上：对账之前节点就可能开始产出
    /// （比如刚建好的会话立刻跑了一轮），只在 `observe_node` 里起消费者会漏掉
    /// 那一段——漏掉的事件不会自己回来，控制面看到的就是一个没有转写的会话。
    /// 一条连接一个消费者；连接断开时广播关闭，任务自然退出（不留孤儿）。
    pub async fn attach_connection(self: &Arc<Self>, connection: Arc<NodeConnection>) {
        let node_id = connection.node_id().to_string();
        let events = connection.subscribe();
        self.connections
            .lock()
            .await
            .insert(node_id.clone(), connection);
        self.start_event_consumer(node_id, events);
    }

    /// 起一个事件消费者（见 `attach_connection` 的顺序理由）。
    fn start_event_consumer(
        self: &Arc<Self>,
        node_id: String,
        mut events: broadcast::Receiver<NodeEvent>,
    ) {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        me.apply_node_event(&node_id, &event).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        // 慢订阅者：视图已经不新鲜了，但**不假装**它还对——
                        // 记下来，让下一次对账把它纠正过来。
                        eprintln!(
                            "node-link: 节点 {node_id} 的事件流落后 {skipped} 条；视图可能不新鲜，等下一次对账"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// 丢掉一条连接（断开时调用）。返回是否有过这条连接。
    pub async fn detach_connection(&self, node_id: &str) -> bool {
        self.connections.lock().await.remove(node_id).is_some()
    }

    /// 取某节点的在线连接（不在线 → `None`）。
    pub async fn connection_of(&self, node_id: &str) -> Option<Arc<NodeConnection>> {
        self.connections.lock().await.get(node_id).cloned()
    }

    /// 在节点上新建一个会话，并把它登记进投影。
    ///
    /// **节点离线就如实拒绝**（d1：不排队、不建占位）——占位会让操作者以为会话
    /// 已经在那儿了，而它其实什么都没发生。
    pub async fn spawn_on(
        &self,
        node_id: &str,
        project: Option<&crate::node_link::placement::ProjectRef>,
        agent_kind: Option<&str>,
        model: Option<&str>,
        mode: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<(ChannelKey, crate::node_link::placement::Placed), PlacementError> {
        let connection = self.connection_of(node_id).await.ok_or_else(|| {
            PlacementError::NodeOffline {
                node_id: node_id.to_string(),
            }
        })?;
        let project_id = project.map(|p| p.id.as_str());
        let session_id = RemoteSessionId::issue(project_id);
        let project_dir = project.map(|p| p.path.clone());
        let placed = crate::node_link::placement::spawn_on(
            &connection,
            session_id,
            project_dir.as_deref(),
            agent_kind,
            model,
            mode,
        )
        .await?;
        let key = self
            .track(
                node_id,
                placed.placement.session_id.as_str(),
                Meta0 {
                    desired_mode: mode.map(str::to_string),
                    effective_mode: placed.mode.clone(),
                    agent_kind: Some(placed.agent_kind.clone()),
                    model: placed.model.clone(),
                    ..Meta0::web(project_dir, prompt.map(str::to_string))
                },
            )
            .await;
        // 首条输入**必须真的送过去**：`Spawn` 只建会话，不带输入。少了这一步
        // 节点上会留下一个"建好但什么都没跑"的会话——工作台显示一切正常，而用户
        // 的第一句话凭空消失了。（顺序也不能反：送输入要走跟踪表找到节点。）
        if let Some(text) = prompt.filter(|t| !t.trim().is_empty())
            && let Err(source) = self
                .prompt(placed.placement.session_id.as_str(), text)
                .await
        {
            // 会话已经建起来了、输入没送到：如实回报失败，不静默吞掉那句话。
            // 行保留着（节点上确实有这个会话），操作者能看到它的真实处境。
            return Err(PlacementError::Link {
                node_id: node_id.to_string(),
                source,
            });
        }
        Ok((key, placed))
    }

    /// 把一轮输入投给远端会话。
    pub async fn prompt(&self, session_id: &str, text: &str) -> Result<(), NodeLinkError> {
        self.request_for(
            session_id,
            SessionOp::Prompt {
                session_id: session_id.to_string(),
                text: text.to_string(),
            },
        )
        .await
    }

    /// 取消远端会话在飞的 turn。
    pub async fn cancel(&self, session_id: &str) -> Result<(), NodeLinkError> {
        self.request_for(
            session_id,
            SessionOp::Cancel {
                session_id: session_id.to_string(),
            },
        )
        .await
    }

    /// 关闭远端会话。
    pub async fn close(&self, session_id: &str) -> Result<(), NodeLinkError> {
        self.request_for(
            session_id,
            SessionOp::Close {
                session_id: session_id.to_string(),
            },
        )
        .await
    }

    /// 期望模型变更。返回节点回报的**实际生效**值（拿不到就不编）。
    pub async fn set_model(
        &self,
        session_id: &str,
        model: &str,
    ) -> Result<Option<String>, NodeLinkError> {
        let result = self
            .send_for(
                session_id,
                SessionOp::SetModel {
                    session_id: session_id.to_string(),
                    model_id: model.to_string(),
                },
            )
            .await?;
        match result {
            // 实际生效值以节点回报为准；节点没报就返回 None（不把期望值回显成已生效）。
            SessionResult::ModelSet { model } => Ok(model),
            _ => Ok(None),
        }
    }

    /// （add-agent-mode-selection）期望模式变更：复用节点链路既有的
    /// `SessionOp::SetMode`（此前没有任何控制面调用方）。返回节点回报的
    /// **实际生效** mode（节点强制不了时如实 `None`，不把期望值回显成
    /// 已生效）。节点接受后同步更新投影 meta 的 desired/effective——
    /// 快照即时反映期望值，实际生效值随事件/对账校正。
    pub async fn set_mode(
        &self,
        session_id: &str,
        mode: &str,
    ) -> Result<Option<String>, NodeLinkError> {
        let result = self
            .send_for(
                session_id,
                SessionOp::SetMode {
                    session_id: session_id.to_string(),
                    mode: mode.to_string(),
                },
            )
            .await?;
        let effective = match result {
            // 节点复用 `ModelSet` 的形状回报实际生效值（见 sebas-node 的
            // SetMode 处理器）。
            SessionResult::ModelSet { model } => model,
            _ => None,
        };
        {
            let mut metas = self.meta.lock().await;
            if let Some(entry) = metas.get_mut(session_id) {
                entry.desired_mode = Some(mode.to_string());
                entry.effective_mode = effective.clone();
            }
        }
        Ok(effective)
    }

    /// 把请求送到该会话所在的节点，并把节点的**拒绝**翻成可判别的错误。
    ///
    /// 这里刻意不吞拒绝：节点说"不"是一个正常结果，调用方（core 通道）要把它
    /// 原样变成 typed rejection 交给客户端，而不是笼统的"失败"。
    async fn request_for(&self, session_id: &str, op: SessionOp) -> Result<(), NodeLinkError> {
        self.send_for(session_id, op).await.map(|_| ())
    }

    /// 送一个操作到该会话所在节点，返回节点的应答（拒绝翻成错误）。
    async fn send_for(
        &self,
        session_id: &str,
        op: SessionOp,
    ) -> Result<SessionResult, NodeLinkError> {
        let node_id = self
            .fleet
            .lock()
            .await
            .node_of(session_id)
            .map(str::to_string)
            .ok_or_else(|| NodeLinkError::Transport {
                cause: format!("会话 {session_id} 不在任何节点的跟踪表里"),
            })?;
        let connection = self.connection_of(&node_id).await.ok_or_else(|| {
            NodeLinkError::Disconnected {
                cause: format!("节点 {node_id} 当前离线，操作未送达"),
            }
        })?;
        match connection.request(op).await? {
            SessionResult::Rejected { code, cause } => Err(NodeLinkError::Transport {
                cause: format!("节点 {node_id} 拒绝（{}）：{cause}", code.as_str()),
            }),
            other => Ok(other),
        }
    }

    /// 请**节点自己**判定一个路径（8.1：路径可用性由项目命名的那台机器判定）。
    ///
    /// 节点离线就没有答案——如实说"够不着"，绝不回退成本地主控的 `stat`：那会把
    /// 「主控上恰好有个同名目录」当成「节点上存在」。
    pub async fn check_path(
        &self,
        node_id: &str,
        path: &str,
    ) -> Result<(bool, bool), NodeLinkError> {
        let connection = self.connection_of(node_id).await.ok_or_else(|| {
            NodeLinkError::Disconnected {
                cause: format!("节点 {node_id} 当前离线，无法校验路径 {path}"),
            }
        })?;
        match connection
            .request(SessionOp::CheckPath {
                path: path.to_string(),
            })
            .await?
        {
            SessionResult::PathChecked { exists, is_dir } => Ok((exists, is_dir)),
            SessionResult::Rejected { code, cause } => Err(NodeLinkError::Transport {
                cause: format!(
                    "节点 {node_id} 拒绝校验路径 {path}（{}）：{cause}",
                    code.as_str()
                ),
            }),
            other => Err(NodeLinkError::Transport {
                cause: format!("校验路径得到非预期应答：{other:?}"),
            }),
        }
    }

    /// 订阅远端审批请求（与本地执行体的审批同一个消费口）。
    pub fn notice_feed(&self) -> broadcast::Receiver<PermissionNotice> {
        self.notices.subscribe()
    }

    /// 某个悬空请求在哪个会话上（按 `request_id` 反查；找不到 → `None`）。
    pub async fn session_of_request(&self, request_id: &str) -> Option<String> {
        let fleet = self.fleet.lock().await;
        for id in fleet.session_ids() {
            if let Some(view) = fleet.view(&id)
                && view
                    .parked_approvals()
                    .iter()
                    .any(|p| p.request_id == request_id)
            {
                return Some(id);
            }
        }
        None
    }

    /// 把一个操作者决定送回节点（6.4 的远端一半）。
    ///
    /// 返回**节点是否真的生效了**：会话已经关闭时节点会回 `applied: false`
    /// （迟到的决定被丢弃），这与"链路失败"是两回事，所以分开表达。
    pub async fn answer_approval(
        &self,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<bool, NodeLinkError> {
        let session_id = self
            .session_of_request(request_id)
            .await
            .ok_or_else(|| NodeLinkError::Transport {
                cause: format!("没有会话在等审批 {request_id}（已回答、未知，或已被节点丢弃）"),
            })?;
        let result = self
            .send_for(
                &session_id,
                SessionOp::ApprovalAnswer {
                    session_id: session_id.clone(),
                    request_id: request_id.to_string(),
                    decision: approval_decision(decision),
                },
            )
            .await?;
        match result {
            SessionResult::ApprovalApplied { applied } => Ok(applied),
            _ => Ok(false),
        }
    }

    /// 把当前**所有悬空请求**广播一遍，让刚回来的操作者看到它们在等什么。
    ///
    /// 重连后调用（`observe_node` 的对账末段）：parked 的请求在节点上一直躺着，
    /// 控制面缺席期间它们不会自己冒出来，所以这里主动播一次。
    pub async fn announce_parked(&self) -> usize {
        let mut count = 0;
        // 先把 id 收进 Vec 再遍历：`for x in self.fleet.lock().await.…` 里那个
        // MutexGuard 临时量会活到**整个循环结束**，而循环体里又要拿同一把锁
        // （`parked`/`node_of`）——非重入的 tokio 互斥量会当场死锁，且是永久挂起。
        let ids = self.fleet.lock().await.session_ids();
        for session_id in ids {
            let parked = self.parked(&session_id).await;
            let key = {
                let fleet = self.fleet.lock().await;
                fleet
                    .node_of(&session_id)
                    .map(|n| ChannelKey::new("web", row_reference(n, &session_id)))
            };
            let Some(key) = key else { continue };
            for approval in parked {
                let _ = self.notices.send(notice_of(&key, &approval));
                count += 1;
            }
        }
        count
    }

    /// 该节点上一次不可达的成因（在线则为 `None`）。
    pub async fn cause_of(&self, node_id: &str) -> Option<String> {
        self.node_cause.lock().await.get(node_id).cloned()
    }

    async fn publish_updated(&self, session_id: &str) {
        if let Some(row) = self.row_of(session_id).await {
            let _ = self.events.send(SessionEvent::Updated { session: row });
        }
    }

    /// 组装一行。**锁序：fleet → meta**。
    async fn row_of(&self, session_id: &str) -> Option<SessionInfo> {
        let fleet = self.fleet.lock().await;
        let node_id = fleet.node_of(session_id)?.to_string();
        let lifecycle = fleet.lifecycle(session_id)?.clone();
        let view = fleet.view(session_id)?;
        let metas = self.meta.lock().await;
        let meta = metas.get(session_id).cloned().unwrap_or_default();
        let causes = self.node_cause.lock().await;

        let parked = view.parked_approvals().len();
        // 相位以**节点事件流**（`view`）为准，生命周期只回答"链路通不通 / 终止了没"。
        // 拿生命周期里的相位当真相会让一个已经结束的会话继续显示成在跑。
        let node_phase = view.phase();
        let (status, phase, node_status, cause) = match &lifecycle {
            SessionLifecycle::Live { .. } => {
                if parked > 0 {
                    // 在等人批 → **不是**在跑（spec：等待不得呈现为运行中）。
                    ("dormant", None, "online", None)
                } else if node_phase == "active" {
                    ("active", Some("OnIt".to_string()), "online", None)
                } else if is_terminal_phase(node_phase) {
                    ("dormant", Some("DONE".to_string()), "online", None)
                } else {
                    ("dormant", None, "online", None)
                }
            }
            SessionLifecycle::NodeOffline { node_id } => (
                "dormant",
                None,
                "offline",
                Some(
                    causes
                        .get(node_id)
                        .cloned()
                        .unwrap_or_else(|| format!("节点 {node_id} 暂时联系不上")),
                ),
            ),
            SessionLifecycle::Terminated { cause: why } => (
                "dormant",
                Some("CrossMark".to_string()),
                "terminated",
                Some(why.clone()),
            ),
        };

        let last_active_unix = meta.last_active_unix.max(
            view.entries()
                .last()
                .map(|e| e.at_unix)
                .unwrap_or(0),
        );
        let channel = if meta.channel.is_empty() {
            "web".to_string()
        } else {
            meta.channel.clone()
        };

        Some(SessionInfo {
            channel,
            key: row_reference(&node_id, session_id),
            session_id: Some(session_id.to_string()),
            status: status.to_string(),
            phase,
            user_prompt: meta.prompt.clone(),
            last_active_unix,
            project_dir: meta.project_dir.clone(),
            current_model: meta.model.clone(),
            available_models: None,
            agent_kind: meta.agent_kind.clone(),
            usage: None,
            backend: None,
            pending: Vec::new(),
            // （add-agent-mode-selection）远端会话的 desired/effective 也随
            // 顶层字段下发（与 remote 视图同值）——前端对两种放置路径用同
            // 一个呈现通道；remote 视图保留节点维度。
            desired_mode: meta.desired_mode.clone(),
            effective_mode: meta.effective_mode.clone(),
            remote: Some(RemoteSessionView {
                node_id,
                node_status: node_status.to_string(),
                node_cause: cause,
                desired_mode: meta.desired_mode.clone(),
                effective_mode: meta.effective_mode.clone(),
                parked_approvals: parked as u32,
                desired_provider: meta.desired_provider.clone(),
                provider: meta.provider.clone(),
                provider_cause: meta.provider_cause.clone(),
            }),
            // rail-declutter-unread：远端会话的段计数不在本路径（节点侧日志
            // 词表与 TurnEntry 不同）——如实记 0，远端行的徽标恒不亮。已知
            // 局限，待节点链路透出统一口径后再接。
            msg_count: 0,
        })
    }
}

/// [`RemoteProjection::track`] 的入参（避免调用方去实现私有类型）。
#[derive(Debug, Clone)]
pub struct Meta0 {
    /// 来源渠道（工作台 `web` / 飞书 `feishu`）。
    pub channel: String,
    /// 项目目录（节点上的路径）。
    pub project_dir: Option<String>,
    /// 首轮提示词（有就展示，没有就不编）。
    pub prompt: Option<String>,
    /// 期望的 mode。
    pub desired_mode: Option<String>,
    /// 下发时**实际生效**的 mode（节点回的）。
    pub effective_mode: Option<String>,
    /// 实际生效的 agent kind。
    pub agent_kind: Option<String>,
    /// 实际生效的模型。
    pub model: Option<String>,
    /// 期望的 provider profile（7.1）。
    pub desired_provider: Option<String>,
    /// 实际生效的 provider profile。
    pub provider: Option<String>,
    /// 应用不上时的成因。
    pub provider_cause: Option<String>,
}

impl Meta0 {
    /// 工作台在远端节点上新建会话时的默认元数据。
    pub fn web(project_dir: Option<String>, prompt: Option<String>) -> Self {
        Self {
            channel: "web".into(),
            project_dir,
            prompt,
            desired_mode: None,
            effective_mode: None,
            agent_kind: None,
            model: None,
            desired_provider: None,
            provider: None,
            provider_cause: None,
        }
    }

    fn into_meta(self) -> Meta {
        Meta {
            channel: self.channel,
            project_dir: self.project_dir,
            prompt: self.prompt,
            desired_mode: self.desired_mode,
            effective_mode: self.effective_mode,
            agent_kind: self.agent_kind,
            model: self.model,
            desired_provider: self.desired_provider,
            provider: self.provider,
            provider_cause: self.provider_cause,
            last_active_unix: 0,
        }
    }
}

/// 工作台侧那张审批卡认的是**编码后的会话键**（`routes::encode_session_key`）。
///
/// 那张函数是 `pub(crate)`，跨 crate 取不到；这里用同一算法（urlencoded
/// `channel\0reference`）现算一份，并有测试钉住形状。真源仍然只有一个：两边都
/// 只是把 `(channel, reference)` 拼成 URL 安全串。
fn encode_session_key(key: &ChannelKey) -> String {
    urlencoding::encode(&format!("{}\0{}", key.channel.as_str(), key.reference)).into_owned()
}

/// 节点侧的审批请求 → 工作台的审批通知。
fn notice_of(key: &ChannelKey, approval: &ParkedApproval) -> PermissionNotice {
    PermissionNotice {
        request_id: approval.request_id.clone(),
        session_id: encode_session_key(key),
        tool_name: approval.tool.clone(),
        args: serde_json::json!({ "category": format!("{:?}", approval.category) }),
        reason: "远端会话上的受门控动作，控制面缺席期间一直悬空".into(),
    }
}

/// 工作台的审批词表 → 节点协议的审批词表。
///
/// 两套词表是**同一个决定**的两种拼写（`AllowOnce`/`allow`），这里显式翻译而不是
/// 靠 serde 变体名碰巧对上——拼错会变成"点允许却什么都没发生"。
fn approval_decision(decision: PermissionDecision) -> ApprovalDecision {
    match decision {
        PermissionDecision::AllowOnce => ApprovalDecision::AllowOnce,
        PermissionDecision::AllowSession => ApprovalDecision::AllowSession,
        PermissionDecision::Deny => ApprovalDecision::Deny,
        // 升级（escalate）在节点侧没有对应语义：**不假装是允许**，落到拒绝，
        // 由控制面把"升级"当成本地动作处理（节点只有 allow/deny 两种结论）。
        PermissionDecision::Escalate { .. } => ApprovalDecision::Deny,
    }
}

/// 节点事件指向哪个会话。
fn event_session_id(event: &NodeEvent) -> Option<&str> {
    match event {
        NodeEvent::TurnBatch { session_id, .. }
        | NodeEvent::State { session_id, .. }
        | NodeEvent::Exited { session_id, .. }
        | NodeEvent::Reclaimed { session_id, .. }
        | NodeEvent::EpochChanged { session_id, .. }
        | NodeEvent::ApprovalRequested { session_id, .. }
        | NodeEvent::GateResolved { session_id, .. } => Some(session_id.as_str()),
        NodeEvent::MaterialsChanged { .. } => None,
    }
}

/// 节点日志条目 → 工作台转写条目。
///
/// 位置直接用日志的 `seq`：这样控制面看到的缺口与 `reclaimed_through` / `gaps`
/// 是同一套编号，不需要第二套映射去解释。
/// 时间戳取条目**落账时**的时间，而不是"现在"——重新回拉一段旧历史时，把它标成
/// 刚刚发生就是伪造。
pub fn turn_entry_of(entry: &LogEntry) -> TurnEntry {
    let (kind, element_type) = match entry.kind.as_str() {
        "prompt" => ("prompt", "markdown"),
        "thinking" => ("content", "thinking"),
        "error" | "spawn_failed" | "failed" => ("content", "error"),
        // 其余（`output`/`audit`/`approval_requested`/`state`/`materials_pinned`…）
        // 都是节点写下的人类可读整句。把它们渲染成正文而不是丢掉：静默吞掉事实
        // 比多显示几行更糟。
        _ => ("content", "markdown"),
    };
    TurnEntry {
        position: entry.seq,
        kind: kind.into(),
        element_type: element_type.into(),
        content: entry.text.clone(),
        created_at_unix: entry.at_unix.max(0) as u64,
    }
}

/// 问节点它现在持有哪些会话。
async fn list_sessions(
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

/// 把投影接到链路生命周期上的观察者（core 装配时挂载）。
pub struct ProjectionObserver {
    projection: Arc<RemoteProjection>,
}

impl ProjectionObserver {
    /// 包装一个投影。
    pub fn new(projection: Arc<RemoteProjection>) -> Arc<dyn ConnectionObserver> {
        Arc::new(Self { projection })
    }
}

#[async_trait::async_trait]
impl ConnectionObserver for ProjectionObserver {
    async fn connected(&self, _node_id: &str, connection: Arc<NodeConnection>) {
        // 先把句柄记下来，再对账：对账里的回拉要经这条连接，而会话操作也从此
        // 有路可走——顺序反了会让"已接入"的会话暂时够不着。
        self.projection
            .attach_connection(Arc::clone(&connection))
            .await;
        let node_id = connection.node_id();
        match self.projection.observe_node(&connection).await {
            // 对账结果如实打出来：操作者要能回答"主控现在认为节点上有什么"，
            // 而不是只有一个"连上了"。
            Ok(report) => eprintln!(
                "node-link: 节点 {} 对账完成：恢复 {} 个会话，判定终止 {} 个，悬空审批 {} 条",
                node_id,
                report.resumed.len(),
                report.terminated.len(),
                report.parked_approvals
            ),
            // 对账失败不该让链路白连：如实记下，链路照常服务（会话操作仍可用），
            // 下一次重连/对账会再试。绝不假装"已同步"。
            Err(e) => eprintln!("node-link: 节点 {node_id} 接入后的对账失败：{e}"),
        }
    }

    async fn disconnected(&self, node_id: &str, cause: Option<String>) {
        // 先摘句柄再标离线：留着一条已经死掉的连接只会让后续操作"送出去但没人接"。
        self.projection.detach_connection(node_id).await;
        let cause = cause.unwrap_or_else(|| "链路已断开".into());
        let affected = self.projection.set_node_offline(node_id, cause).await;
        if affected > 0 {
            eprintln!("node-link: 节点 {node_id} 断开，{affected} 个会话暂时看不见（未终止）");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sebas_node_link::{GateCategory, LogEntry, SessionMode};

    fn log_entry(seq: u64, kind: &str, text: &str, at_unix: i64) -> LogEntry {
        LogEntry {
            seq,
            kind: kind.into(),
            text: text.into(),
            at_unix,
            data: None,
        }
    }

    fn batch(session_id: &str, from_seq: u64, entries: Vec<LogEntry>) -> NodeEvent {
        NodeEvent::TurnBatch {
            session_id: session_id.into(),
            epoch: 1,
            from_seq,
            entries,
            coalesced_overflow: false,
        }
    }

    #[test]
    fn row_reference_is_stable_and_parses_back() {
        let r = row_reference("dev-box", "proj-a:1a2b");
        assert_eq!(parse_reference(&r), Some(("dev-box", "proj-a:1a2b")));
        // 确定性：同一个会话在控制面重启后还是同一行（否则每次重启都多一批"新会话"）。
        assert_eq!(r, row_reference("dev-box", "proj-a:1a2b"));
        // 非远端行如实返回 None，不去猜。
        assert_eq!(parse_reference("oc_chat"), None);
        assert_eq!(parse_reference("node\0dev-box"), None);
    }

    #[tokio::test]
    async fn a_live_session_shows_its_node_mode_and_running_state() {
        let p = RemoteProjection::new();
        let key = p
            .track(
                "dev-box",
                "proj-a:1",
                Meta0 {
                    desired_mode: Some("ask".into()),
                    effective_mode: Some("ask".into()),
                    ..Meta0::web(Some("/srv/repo".into()), Some("do it".into()))
                },
            )
            .await;

        let row = p.row(&key).await.expect("行应当可见");
        let remote = row.remote.expect("远端行必须带节点维度");
        assert_eq!(remote.node_id, "dev-box");
        assert_eq!(remote.node_status, "online");
        assert_eq!(remote.node_cause, None);
        assert_eq!(remote.desired_mode.as_deref(), Some("ask"));
        assert_eq!(remote.effective_mode.as_deref(), Some("ask"));
        assert_eq!(remote.parked_approvals, 0);
        assert_eq!(row.project_dir.as_deref(), Some("/srv/repo"));
        assert_eq!(row.session_id.as_deref(), Some("proj-a:1"));

        // 节点说它在跑 → 控制面就显示在跑。
        p.apply_node_event(
            "dev-box",
            &NodeEvent::State {
                session_id: "proj-a:1".into(),
                phase: "active".into(),
                detail: None,
            },
        )
        .await;
        assert_eq!(p.row(&key).await.unwrap().status, "active");
    }

    #[tokio::test]
    async fn a_live_session_still_shows_everything_when_the_node_is_gone() {
        let p = RemoteProjection::new();
        let key = p
            .track("dev-box", "proj-a:1", Meta0::web(None, None))
            .await;
        p.apply_node_event(
            "dev-box",
            &NodeEvent::State {
                session_id: "proj-a:1".into(),
                phase: "active".into(),
                detail: None,
            },
        )
        .await;

        let affected = p.set_node_offline("dev-box", "链路已断开（对端关闭）").await;
        assert_eq!(affected, 1);
        let row = p.row(&key).await.expect("链路断了会话仍要可见");
        let remote = row.remote.unwrap();
        assert_eq!(remote.node_status, "offline");
        assert_eq!(
            remote.node_cause.as_deref(),
            Some("链路已断开（对端关闭）"),
            "成因要如实带出来，不能只说『离线』"
        );
        // **离线不是终止**（rung ③⁺）：会话还活着，只是暂时看不见。
        assert!(
            p.fleet
                .lock()
                .await
                .lifecycle("proj-a:1")
                .unwrap()
                .is_alive()
        );
        assert_eq!(row.status, "dormant", "看不见的时候不该显示成在跑");

        // 节点回来：成因清掉，视图重新变为在线。
        p.set_node_online("dev-box").await;
        let row = p.row(&key).await.unwrap();
        assert_eq!(row.remote.unwrap().node_status, "online");
        assert_eq!(p.cause_of("dev-box").await, None);
    }

    #[tokio::test]
    async fn a_parked_session_reads_as_waiting_not_running() {
        let p = RemoteProjection::new();
        let key = p
            .track("dev-box", "proj-a:1", Meta0::web(None, None))
            .await;
        p.apply_node_event(
            "dev-box",
            &NodeEvent::State {
                session_id: "proj-a:1".into(),
                phase: "active".into(),
                detail: None,
            },
        )
        .await;
        assert_eq!(p.row(&key).await.unwrap().status, "active");

        // 一条受门控动作停住了：节点上报请求，控制面还没决定。
        p.apply_node_event(
            "dev-box",
            &NodeEvent::ApprovalRequested {
                session_id: "proj-a:1".into(),
                request_id: "proj-a:1:req-1".into(),
                tool: "bash".into(),
                category: GateCategory::Execute,
                mode: SessionMode::Ask,
            },
        )
        .await;
        let row = p.row(&key).await.unwrap();
        assert_eq!(
            row.remote.as_ref().unwrap().parked_approvals,
            1,
            "悬空请求数要能数出来"
        );
        assert_eq!(
            row.status, "dormant",
            "在等人批的会话不得呈现为运行中（spec：等待 ≠ 运行中）"
        );
        assert_eq!(p.parked("proj-a:1").await.len(), 1, "请求本身要可达");

        // 决定下来了：回到在跑，请求不再悬空。
        p.apply_node_event(
            "dev-box",
            &NodeEvent::GateResolved {
                session_id: "proj-a:1".into(),
                request_id: "proj-a:1:req-1".into(),
                decision: "allow".into(),
                source: "control-plane".into(),
            },
        )
        .await;
        let row = p.row(&key).await.unwrap();
        assert_eq!(row.status, "active");
        assert_eq!(row.remote.unwrap().parked_approvals, 0);
    }

    #[tokio::test]
    async fn node_reported_exit_is_a_termination_with_the_nodes_cause() {
        let p = RemoteProjection::new();
        let key = p
            .track("dev-box", "proj-a:1", Meta0::web(None, None))
            .await;
        p.apply_node_event(
            "dev-box",
            &NodeEvent::Exited {
                session_id: "proj-a:1".into(),
                cause: "子进程退出码 1".into(),
            },
        )
        .await;
        let row = p.row(&key).await.unwrap();
        let remote = row.remote.unwrap();
        // `node_status` 讲的是**这个会话相对节点**的可判定状态：节点还在线，但这
        // 个会话已经结束了——所以是 terminated，而不是 online。
        assert_eq!(remote.node_status, "terminated");
        assert!(
            remote.node_cause.as_deref().unwrap_or_default().contains("退出码 1"),
            "终止成因来自节点：{:?}",
            remote.node_cause
        );
        assert_eq!(row.status, "dormant");
        // 终止的成因来自节点，措辞不该与「暂时联系不上」混为一谈。
        assert!(
            p.fleet
                .lock()
                .await
                .lifecycle("proj-a:1")
                .unwrap()
                .is_alive()
                == false
        );
    }

    #[tokio::test]
    async fn batches_become_transcript_entries_with_the_nodes_timestamps() {
        let p = RemoteProjection::new();
        let key = p
            .track("dev-box", "proj-a:1", Meta0::web(None, None))
            .await;
        let entries = vec![
            log_entry(1, "prompt", "fix the bug", 1_700_000_000),
            log_entry(2, "output", "done", 1_700_000_010),
        ];
        p.apply_node_event("dev-box", &batch("proj-a:1", 1, entries.clone()))
            .await;

        let turns = p.turns("proj-a:1", 1).await;
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].position, 1);
        assert_eq!(turns[0].kind, "prompt");
        assert_eq!(turns[1].kind, "content");
        assert_eq!(
            turns[1].created_at_unix, 1_700_000_010,
            "时间戳取条目落账时的时间，不是『现在』——否则回拉旧历史会被标成刚发生"
        );
        // 按位置回拉：只要后半段。
        let tail = p.turns("proj-a:1", 2).await;
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].position, 2);

        // 幂等（5.1 的验收点）：同一批再来一遍，结果逐字不变。
        let before = p.row(&key).await.unwrap();
        p.apply_node_event("dev-box", &batch("proj-a:1", 1, entries))
            .await;
        let after = p.row(&key).await.unwrap();
        assert_eq!(before, after, "重复应用同一段不得改变视图");
        assert_eq!(p.turns("proj-a:1", 1).await.len(), 2);
    }

    #[tokio::test]
    async fn an_event_for_an_untracked_session_is_adopted_not_dropped() {
        // 控制面重启后事件可能早于快照到达：这段事实不能被丢掉。
        let p = RemoteProjection::new();
        p.apply_node_event(
            "dev-box",
            &NodeEvent::State {
                session_id: "proj-a:9".into(),
                phase: "active".into(),
                detail: None,
            },
        )
        .await;
        let rows = p.rows().await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id.as_deref(), Some("proj-a:9"));
        assert_eq!(rows[0].remote.as_ref().unwrap().node_id, "dev-box");
    }

    #[test]
    fn turn_kinds_map_honestly() {
        let e = turn_entry_of(&log_entry(7, "error", "boom", 5));
        assert_eq!(e.element_type, "error");
        assert_eq!(e.kind, "content");
        let e = turn_entry_of(&log_entry(8, "thinking", "hmm", 5));
        assert_eq!(e.element_type, "thinking");
        // 节点的运行日志（审计/审批/状态）不丢弃：它们是节点写下的人类可读整句。
        let e = turn_entry_of(&log_entry(9, "audit", "mode=ask 放行 bash", 5));
        assert_eq!(e.element_type, "markdown");
        assert_eq!(e.content, "mode=ask 放行 bash");
    }

    #[tokio::test]
    async fn remote_target_rejects_a_session_on_another_node() {
        let p = RemoteProjection::new();
        p.track("dev-box", "proj-a:1", Meta0::web(None, None)).await;
        let ok = ChannelKey::new("web", row_reference("dev-box", "proj-a:1"));
        assert_eq!(
            p.remote_target(&ok).await,
            Some(("dev-box".to_string(), "proj-a:1".to_string()))
        );
        // 节点对不上的键不是这个会话（否则两台机器上的同名 id 会互相冒充）。
        let wrong = ChannelKey::new("web", row_reference("other-box", "proj-a:1"));
        assert_eq!(p.remote_target(&wrong).await, None);
        // 渠道也对不上：本地会话的键根本没有 node 前缀。
        assert_eq!(
            p.remote_target(&ChannelKey::new("web", "oc_chat")).await,
            None
        );
    }
}
