use crate::error::DispatchError;
use sebas_channels::ChannelKey;
use sebas_domain::session::{SessionMode, SessionPhase};
use sebas_models::session_map::SessionMapRow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;

/// A queued turn waiting to be processed by the per-turn reply handler.
/// `id` is assigned by [`SessionMap::enqueue_turn`] from the per-key monotonic
/// counter (workbench-turn-queue D2); call sites construct via
/// [`QueuedTurn::new`] and never pick ids themselves.
#[derive(Debug, Clone)]
pub struct QueuedTurn {
    pub id: u64,
    pub prompt: String,
    pub reply_to: Option<String>,
    pub priority: bool,
}

impl QueuedTurn {
    pub fn new(prompt: impl Into<String>, reply_to: Option<String>, priority: bool) -> Self {
        Self {
            id: 0,
            prompt: prompt.into(),
            reply_to,
            priority,
        }
    }
}

/// A submission staged during the spawn window (workbench-turn-queue D1/D2):
/// accepted before the session existed; combined with its siblings into one
/// prompt at activation. Ids come from the same per-key monotonic counter as
/// [`QueuedTurn`].
#[derive(Debug, Clone)]
pub struct StagedSubmission {
    pub id: u64,
    pub text: String,
}

// `PendingDisposition` / `PendingSubmission` 已迁往 `sebas_domain::session`
// （add-domain-layer 3.1，design D3 原位再导出）——它们随 `SessionInfo`
// 上 wire，是会话域的形状而非引擎状态机的一部分。
pub use sebas_domain::session::{PendingDisposition, PendingSubmission};

/// remove/move 的类型化拒绝（design D7）。`Unknown` = id 不在队列里也从未
/// 开始；`AlreadyStarted` = 已开轮/已被合并投递；`PriorityConflict` =
/// 不能越过（或移动）优先项；`OutOfRange` = 目标位置越界。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingOpError {
    Unknown,
    AlreadyStarted,
    PriorityConflict,
    OutOfRange,
}

/// In-memory session mapping state. `Spawning` is a placeholder inserted
/// synchronously when the first text arrives, so a second text racing the
/// (slow) ACP spawn is queued instead of spawning a duplicate child.
/// `Spawning` is never persisted (the child is gone after a restart anyway).
/// `Dormant` is the inverse: a mapping restored from the state file after a
/// daemon restart — the session_id is known but no child process is alive.
/// The first inbound text lazily respawns it (openspec/specs/session-lifecycle/spec.md); `Dormant` never
/// appears at runtime only via `restore_rows` (state-store restore).
///
/// fail-fast-on-startup-errors（webui spec delta）：spawn 失败的会话不再被
/// 静默拆除（Removed），而是保留为 `SpawnFailed` 终态——会话在列表/详情里
/// 保持可见、状态标记 spawn-failed，transcript 以合成 id 持有一条 inline
/// 错误。`SpawnFailed` 同样不入盘。
#[derive(Debug, Clone)]
pub enum MappingState {
    /// 占位/spawn-in-flight。`awaiting_first_prompt` 标记 0-turn 占位（webui
    /// 经 `web_create_placeholder` 显式创建、还没有首条消息）：它的首条消息
    /// 走 SpawnNew 触发真实 spawn（workbench-agent-wire-fix D1）。普通
    /// spawn-in-flight（首条消息已触发 spawn、子进程还在启动）为 false——
    /// 期间到达的后续文本进 `pending`，activate 后 drain，绝不二次 spawn。
    /// 占位身份必须是显式标记：此前用 `pending_kind/model 非空` 推断占位，
    /// 而默认 agent 的占位两个候选字段都是 None，首条消息被误判成
    /// spawn-in-flight 永久排队（根因修复）。
    Spawning {
        pending: Vec<StagedSubmission>,
        awaiting_first_prompt: bool,
    },
    Active {
        session_id: String,
    },
    Dormant {
        session_id: String,
    },
    /// spawn 失败终态。`session_id` 是 transcript 寻址用的合成 id（"failed-N"），
    /// 不是 live 路由 id；`reason` 是触发方（dispatch）记录的失败原因。
    SpawnFailed {
        session_id: String,
        reason: String,
    },
}

/// SpawnFailed transcript 的合成 id 计数器（进程内唯一即可；不入盘）。
static FAILED_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl MappingState {
    /// 该映射状态对外可见的**会话相位**（type-session-vocabularies 2.2）。
    ///
    /// 返回共享 [`SessionPhase`] 而不是字符串字面量：取值拼写与原字面量逐字
    /// 一致（`active` / `dormant` / `spawning` / `spawn-failed`），但新增一个
    /// `MappingState` 变体会让这里**编译失败**（穷尽匹配），而不是静默漏映射。
    pub fn phase(&self) -> SessionPhase {
        match self {
            Self::Active { .. } => SessionPhase::Active,
            Self::Dormant { .. } => SessionPhase::Dormant,
            Self::Spawning { .. } => SessionPhase::Spawning,
            Self::SpawnFailed { .. } => SessionPhase::SpawnFailed,
        }
    }
}

fn next_failed_id() -> String {
    let n = FAILED_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("failed-{n}")
}

// `SessionIdentity` 已迁往 `sebas_domain::session`（add-domain-layer 3.1）。
// 原 `SessionIdentity::of(&Mapping)` 快照构造在全仓无调用点（归档路径由
// webui::archive 手工构建四项），随迁删除；`is_empty` 在域内保留。
pub use sebas_domain::session::SessionIdentity;

#[derive(Debug, Clone)]
pub struct Mapping {
    pub state: MappingState,
    pub last_active_unix: i64,
    /// Working directory for the project (set when spawned via WebUI).
    /// `None` for Feishu-originated sessions or sessions without a project dir.
    pub project_dir: Option<String>,
    /// Execution-backend kind requested at create time (a 0-turn placeholder
    /// remembers it until the first message triggers the spawn). `None` =
    /// fall back to the configured default kind.
    pub pending_kind: Option<String>,
    /// Model id requested at create time (a 0-turn placeholder remembers it
    /// until the first message triggers the spawn; add-acp-model-selection).
    /// `None` = the agent's default model.
    pub pending_model: Option<String>,
    /// （add-agent-mode-selection）创建时请求的 mode（0-turn 占位记住，首条
    /// 消息触发 spawn 时消费）。词汇 = 控制面 `ask`/`edit`/`allow`/`auto`；
    /// `None` = agent 默认行为（wire 不携带 mode）。
    pub pending_mode: Option<String>,
    /// （type-session-vocabularies 2.4）操作者期望的会话 mode：类型化为共享的
    /// [`SessionMode`]（不再是本层自持的 ad-hoc `String`）。**非空**：缺省即
    /// `ask`（[`SessionMode::Ask`]），不再有 `None` 路径。持久行仍以拼写
    /// （projects.db 的 `desired_mode TEXT`）落盘——磁盘形状不在本 change 的
    /// 改动面内，转换点唯一（`persist_upsert`）。
    pub desired_mode: SessionMode,
    /// （add-agent-mode-selection）执行体回报的**实际生效** mode（本机 =
    /// spawn argv 实际应用值 / `ModeChanged` 事件；远端 = 节点回报）。
    /// `None` = 执行体未声称任何 mode 生效（如实呈现 desired/effective 差异）。
    pub effective_mode: Option<SessionMode>,
    /// The agent's real ACP session id when it differs from the routing id
    /// (native-ACP agents, e.g. opencode; the `session/new` id on a fresh
    /// spawn, the loaded conversation id on a successful resume). `None` for
    /// Claude (routing id == conversation id) and legacy records. Persisted
    /// with the session record; a resume reads it to load the conversation
    /// by the id the agent actually knows (acp-session-mapping D3).
    pub acp_session_id: Option<String>,
    /// 会话当前的模型 id（add-acp-model-selection）：spawn 时由 driver 上报的
    /// configOptions 填充；SetModel 成功后更新。`None` = agent 无模型选项。
    /// 随映射落状态库（session_map.current_model 列）。
    pub current_model: Option<String>,
    /// 该 ACP 会话可选的模型 id 列表（来自 agent 的 configOptions）。webui
    /// 创建会话下拉的数据源；`None`/空 = 无模型选择面。
    pub available_models: Option<Vec<String>>,
    /// （session-slash-commands 2.1）agent 自广告的会话命令表：claude 的
    /// `get_server_info()` 握手 / 通用 ACP 的 `available_commands_update`，
    /// 经 `AcpEvent::AvailableCommands` 物化到此（二次通知覆盖旧表）。空表
    /// = 无命令面板（native 等无发现能力的会话恒空，诚实退化非错误）。
    /// 内存层字段，不落盘——重连后 agent 重新广告即可重建。
    pub available_commands: Vec<sebas_acp::AvailableCommand>,
    /// （fix-webui-approval-restore-and-session-identity 5.1，design D6）操作者
    /// 设置的会话 label。`None` = 未设置（命名回退首条 prompt 预览 / 短 id）。
    /// 随映射行落库——label 跨重启保持。
    pub label: Option<String>,
    /// （fix-webui-qa-defects-round4 1.2/3.1）命名来源迁移位：首条 prompt 预览。
    /// 活跃会话的预览以卡态（`user_prompt`）为准；本字段在卡态不存在时兜底
    /// ——归档恢复（快照元数据迁移）与 terminal teardown（卡态丢弃前迁出），
    /// 两处都靠它保住 rail 行名不退化为短 id。`None` = 无迁移来源（旧行为）。
    /// 随映射行落库。
    pub prompt_preview: Option<String>,
}

impl Mapping {
    pub fn active(session_id: impl Into<String>) -> Self {
        Self {
            state: MappingState::Active {
                session_id: session_id.into(),
            },
            last_active_unix: crate::engine::now_unix(),
            project_dir: None,
            pending_kind: None,
            pending_model: None,
            pending_mode: None,
            desired_mode: crate::engine::ask_mode(),
            effective_mode: None,
            acp_session_id: None,
            current_model: None,
            available_models: None,
            available_commands: Vec::new(),
            label: None,
            prompt_preview: None,
        }
    }

    /// [`Mapping::active`] plus the real ACP session id (fresh spawn /
    /// successful resume mapping write — acp-session-mapping D3).
    pub fn active_with_acp(session_id: impl Into<String>, acp_session_id: Option<String>) -> Self {
        Self {
            state: MappingState::Active {
                session_id: session_id.into(),
            },
            last_active_unix: crate::engine::now_unix(),
            project_dir: None,
            pending_kind: None,
            pending_model: None,
            pending_mode: None,
            desired_mode: crate::engine::ask_mode(),
            effective_mode: None,
            acp_session_id,
            current_model: None,
            available_models: None,
            available_commands: Vec::new(),
            label: None,
            prompt_preview: None,
        }
    }

    pub fn spawning() -> Self {
        Self {
            state: MappingState::Spawning {
                pending: Vec::new(),
                awaiting_first_prompt: false,
            },
            last_active_unix: crate::engine::now_unix(),
            project_dir: None,
            pending_kind: None,
            pending_model: None,
            pending_mode: None,
            desired_mode: crate::engine::ask_mode(),
            effective_mode: None,
            acp_session_id: None,
            current_model: None,
            available_models: None,
            available_commands: Vec::new(),
            label: None,
            prompt_preview: None,
        }
    }

    /// A Spawning placeholder created eagerly with a 0-turn session request
    /// (no prompt yet): remember the requested kind/model so the first message
    /// spawns the right agent (0-turn 会话修复，P2）。`mode`（
    /// add-agent-mode-selection）同 model：占位记住、首条消息 spawn 时消费，
    /// 并记为 desired mode 供快照暴露。普通 spawn 流程直接消费时这些字段
    /// 保持 None（走默认 kind / agent 默认模型 / agent 默认行为）。
    /// `awaiting_first_prompt` 是占位身份本身（D1）：只有真正的 0-turn 占位
    /// 才置 true——prompt 已随 spawn 指令直达的真实 spawn 必须传 false，
    /// 否则 spawn 窗口内的后续消息会被误判为占位首条消息而二次 spawn、
    /// 且 in-flight spawn 会以占位身份入盘。
    pub fn spawning_with(
        kind: Option<String>,
        model: Option<String>,
        mode: Option<String>,
        awaiting_first_prompt: bool,
    ) -> Self {
        Self {
            state: MappingState::Spawning {
                pending: Vec::new(),
                awaiting_first_prompt,
            },
            last_active_unix: crate::engine::now_unix(),
            project_dir: None,
            pending_kind: kind,
            pending_model: model,
            pending_mode: mode.clone(),
            // D5b：占位的 desired mode 非空——创建请求未点名时缺省 ask（
            // spawn argv 是否带 flag 由 pending_mode 决定，与 CLI 默认语义
            // 等价，快照上永远有确定的控制面词）。
            desired_mode: mode
                .as_deref()
                .map(SessionMode::from_wire)
                .unwrap_or(SessionMode::Ask),
            effective_mode: None,
            acp_session_id: None,
            current_model: None,
            available_models: None,
            available_commands: Vec::new(),
            label: None,
            prompt_preview: None,
        }
    }

    pub fn dormant(session_id: impl Into<String>, last_active_unix: i64) -> Self {
        Self {
            state: MappingState::Dormant {
                session_id: session_id.into(),
            },
            last_active_unix,
            project_dir: None,
            pending_kind: None,
            pending_model: None,
            pending_mode: None,
            desired_mode: crate::engine::ask_mode(),
            effective_mode: None,
            acp_session_id: None,
            current_model: None,
            available_models: None,
            available_commands: Vec::new(),
            label: None,
            prompt_preview: None,
        }
    }

    /// Live routing id — `Some` only for `Active` (a child process exists).
    /// `Dormant` deliberately returns `None` so liveness checks
    /// (`session_alive`, button-callback routing) treat it as dead.
    /// `SpawnFailed` likewise returns `None` — nothing is live.
    pub fn session_id(&self) -> Option<&str> {
        match &self.state {
            MappingState::Active { session_id } => Some(session_id),
            MappingState::Spawning { .. }
            | MappingState::Dormant { .. }
            | MappingState::SpawnFailed { .. } => None,
        }
    }

    /// Transcript 寻址 id：`Active` 用路由 id；`SpawnFailed` 用合成 id（错误
    /// 条目挂在它下面）；`Dormant` 用持久化的 session_id（fix-webui-qa-defects
    /// 2.1：归档恢复以 Dormant 重建映射并把转写回放进 turn 存储，detail 必须
    /// 能按该 id 取回全部条目——重启恢复的 Dormant 会话 turn_log 本就为空，
    /// 行为不变）。`Spawning` 占位无可寻址 transcript。
    pub fn transcript_id(&self) -> Option<&str> {
        match &self.state {
            MappingState::Active { session_id }
            | MappingState::Dormant { session_id } => Some(session_id),
            MappingState::SpawnFailed { session_id, .. } => Some(session_id),
            MappingState::Spawning { .. } => None,
        }
    }

    /// 0-turn 占位旗标（fix-webui-qa-defects 3.1，design D2）：占位不构成在飞
    /// 回合，停滞看门狗据此跳过（经映射查询，不改卡片 FSM）。
    pub fn awaiting_first_prompt(&self) -> bool {
        matches!(
            &self.state,
            MappingState::Spawning {
                awaiting_first_prompt: true,
                ..
            }
        )
    }

    /// The spawn-failure reason, when this mapping is in the `SpawnFailed`
    /// terminal state.
    pub fn spawn_failed_reason(&self) -> Option<&str> {
        match &self.state {
            MappingState::SpawnFailed { reason, .. } => Some(reason),
            _ => None,
        }
    }

    /// Id worth persisting — `Active` and `Dormant` both survive a restart
    /// (Dormant is what a persisted-then-restored Active becomes).
    fn persisted_id(&self) -> Option<&str> {
        match &self.state {
            MappingState::Active { session_id } | MappingState::Dormant { session_id } => {
                Some(session_id)
            }
            MappingState::Spawning { .. } | MappingState::SpawnFailed { .. } => None,
        }
    }
}

/// What the router should do with an inbound text, decided atomically under
/// a single write lock (no check-then-act window).
#[derive(Debug)]
pub enum TextRoute {
    /// No mapping existed; a Spawning placeholder was inserted. Spawn now.
    SpawnNew,
    /// A live session exists; forward the prompt to this session_id.
    Continue(String),
    /// A restored (Dormant) mapping was claimed: the placeholder is in
    /// place and the caller should lazily respawn the given (old) session_id,
    /// falling back to a fresh session when the agent cannot load it
    /// (openspec/specs/session-lifecycle/spec.md).
    Resume(String),
    /// A spawn is already in flight; the prompt was staged (durable until
    /// activation) and will be combined into the first prompt.
    Enqueued,
    /// The spawn-window staging queue is at its cap: the submission was NOT
    /// accepted. The caller must reject visibly at the submission surface
    /// (design D5 — a submission is either executed or told it wasn't).
    Overflow { cap: usize },
}

/// Outcome of a `/new`-initiated spawn request.
pub enum BeginSpawn {
    /// No mapping existed; placeholder inserted. Caller should spawn.
    Fresh,
    /// An Active session was replaced by the placeholder. Caller should spawn.
    ReplacedActive,
    /// A spawn is already in flight for this key; the pending queue (if any)
    /// is preserved. Caller must NOT emit another SpawnAcp.
    AlreadySpawning,
}

/// 聚焦即拉起的路由结果（`route_activate`，workbench-live-conversation-flow
/// 3.1）。与 `TextRoute` 的差别：绝无 Enqueued/Overflow——激活不入队任何
/// 文本，只决定「要不要把子进程拉起来」。
pub enum ActivateRoute {
    /// Active 映射：子进程活着，激活是幂等无操作。
    AlreadyLive,
    /// Spawning 占位：拉起已在途，无需重复触发。
    AlreadyStarting,
    /// 占位/失败态换到 Spawning，调用方走 fresh spawn（无 prompt）。
    SpawnNew,
    /// Dormant 映射被认领并换到 Spawning；调用方按给定旧 session_id 走
    /// resume（无 prompt；load 被拒时既有回退语义不变）。
    Resume(String),
}

const MAX_PENDING: usize = 16;

/// 分配一个 per-key 单调 pending id（自由函数以便在持有 inner 写锁的窗口
/// 内按字段借用计数器，避免整对象借用冲突）。叶子锁：绝不与其他锁嵌套。
fn alloc_pending_id(counter: &std::sync::Mutex<HashMap<ChannelKey, u64>>, key: &ChannelKey) -> u64 {
    let mut next = counter.lock().unwrap();
    let entry = next.entry(key.clone()).or_insert(0);
    *entry += 1;
    *entry
}

/// staging 队列上限的公开读数（webui/通道层拒绝文案需要点名上限，5.1）。
pub const MAX_PENDING_SUBMISSIONS: usize = MAX_PENDING;

#[derive(Clone)]
pub struct SessionMap {
    inner: Arc<RwLock<HashMap<ChannelKey, Mapping>>>,
    turn_queue: Arc<RwLock<HashMap<ChannelKey, VecDeque<QueuedTurn>>>>,
    /// per-key 单调 pending id 计数器（workbench-turn-queue D2）。独立叶子
    /// 锁：分配 id 时单独持有，绝不与其他锁嵌套——staging（inner 锁）与
    /// turn 队列（turn_queue 锁）两条入队路径共用同一计数序列。
    next_id: Arc<std::sync::Mutex<HashMap<ChannelKey, u64>>>,
    /// 已投递（开轮/激活合并）的 pending id，per-key（D2：id 失效后仍可被
    /// 确定性识别为「已开始」，与「未知 id」区分）。叶子锁：在 inner 写锁
    /// （activate 合并）或 turn_queue 写锁（pop 投递）内写入，读侧在
    /// remove/move 的末段检查。随 `clear_queue`（会话拆除/替换）清空。
    delivered: Arc<std::sync::Mutex<HashMap<ChannelKey, HashSet<u64>>>>,
    capacity: usize,
}

impl SessionMap {
    pub fn new() -> Self {
        Self::with_capacity(usize::MAX)
    }
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            turn_queue: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(std::sync::Mutex::new(HashMap::new())),
            delivered: Arc::new(std::sync::Mutex::new(HashMap::new())),
            capacity: cap,
        }
    }

    /// 分配一个 per-key 单调 pending id。叶子锁内完成（D2）：计数器绝不
    /// 与 inner/turn_queue 锁嵌套，两条入队路径共用同一序列。
    fn alloc_id(&self, key: &ChannelKey) -> u64 {
        alloc_pending_id(&self.next_id, key)
    }

    /// Atomic check+act for an inbound text (D8 fix). Also touches
    /// `last_active_unix` on every hit so the timestamp reflects real use.
    pub async fn route_text(
        &self,
        key: ChannelKey,
        prompt: String,
    ) -> Result<TextRoute, DispatchError> {
        let mut g = self.inner.write().await;
        match g.get_mut(&key) {
            None => {
                if g.len() >= self.capacity {
                    return Err(DispatchError::Capacity(self.capacity));
                }
                g.insert(key, Mapping::spawning());
                Ok(TextRoute::SpawnNew)
            }
            Some(m) => {
                m.last_active_unix = crate::engine::now_unix();
                match &mut m.state {
                    MappingState::Active { session_id } => {
                        Ok(TextRoute::Continue(session_id.clone()))
                    }
                    MappingState::Dormant { session_id } => {
                        // Claim the restored mapping for lazy respawn: swap in a
                        // Spawning placeholder so a concurrent second text queues
                        // instead of double-spawning, and hand the old id to the
                        // caller (openspec/specs/session-lifecycle/spec.md).
                        let old = session_id.clone();
                        m.state = MappingState::Spawning {
                            pending: Vec::new(),
                            awaiting_first_prompt: false,
                        };
                        Ok(TextRoute::Resume(old))
                    }
                    MappingState::SpawnFailed { .. } => {
                        // spawn-failed 会话上的新消息 = 用户重试：清掉失败态、
                        // 回到 Spawning 占位并重新走 spawn（旧错误条目保留在
                        // 原 transcript 下作为历史，视图随新会话推进）。
                        m.state = MappingState::Spawning {
                            pending: Vec::new(),
                            awaiting_first_prompt: false,
                        };
                        Ok(TextRoute::SpawnNew)
                    }
                    MappingState::Spawning {
                        pending,
                        awaiting_first_prompt,
                    } => {
                        if *awaiting_first_prompt {
                            // A 0-turn placeholder awaits its first message:
                            // consume the marker and hand the message to the
                            // spawn path (workbench-agent-wire-fix D1). The
                            // marker — not the optional kind/model fields —
                            // is the placeholder's identity; a placeholder
                            // created without an explicit kind/model used to
                            // lose that inference here and its first message
                            // was queued with nothing to ever drain it.
                            m.state = MappingState::Spawning {
                                pending: Vec::new(),
                                awaiting_first_prompt: false,
                            };
                            Ok(TextRoute::SpawnNew)
                        } else if pending.len() < MAX_PENDING {
                            let id = alloc_pending_id(&self.next_id, &key);
                            pending.push(StagedSubmission { id, text: prompt });
                            Ok(TextRoute::Enqueued)
                        } else {
                            // workbench-turn-queue 5.1：满队列不再静默吞掉——
                            // 返回携带上限的拒绝变体，提交面据此可见拒绝；已
                            // 暂存条目不被顶掉，且拒绝路径不分配 id。
                            tracing::warn!(
                                ?key,
                                cap = MAX_PENDING,
                                "staging queue full; rejecting newest submission"
                            );
                            Ok(TextRoute::Overflow { cap: MAX_PENDING })
                        }
                    }
                }
            }
        }
    }

    /// 聚焦即拉起（workbench-live-conversation-flow 3.1）：为聚焦的会话
    /// 拉起子进程（不带 prompt、不跑首轮）。与 `route_text` 同一套原子
    /// 状态机，但绝不创建新映射、绝不入队：Active = 幂等无操作，Dormant =
    /// 认领并交还原 session_id（resume），占位/失败态按需换到 Spawning。
    /// 返回 `None` = 未知 key（调用方按 404 处置）。
    pub async fn route_activate(&self, key: &ChannelKey) -> Option<ActivateRoute> {
        let mut g = self.inner.write().await;
        match g.get_mut(key) {
            None => None,
            Some(m) => {
                m.last_active_unix = crate::engine::now_unix();
                match &mut m.state {
                    MappingState::Active { .. } => Some(ActivateRoute::AlreadyLive),
                    MappingState::Spawning {
                        pending,
                        awaiting_first_prompt,
                    } => {
                        // 0-turn 占位（awaiting_first_prompt）= 等待启动的空会
                        // 话：激活语义与首条消息相同——消费标记、换到无标记的
                        // Spawning 并走 fresh spawn（route_text 同一语义）。
                        // 已在途的常规 spawn（标记已消费）才是 AlreadyStarting。
                        if *awaiting_first_prompt {
                            m.state = MappingState::Spawning {
                                pending: std::mem::take(pending),
                                awaiting_first_prompt: false,
                            };
                            Some(ActivateRoute::SpawnNew)
                        } else {
                            Some(ActivateRoute::AlreadyStarting)
                        }
                    }
                    MappingState::Dormant { session_id } => {
                        let old = session_id.clone();
                        m.state = MappingState::Spawning {
                            pending: Vec::new(),
                            awaiting_first_prompt: false,
                        };
                        Some(ActivateRoute::Resume(old))
                    }
                    MappingState::SpawnFailed { .. } => {
                        m.state = MappingState::Spawning {
                            pending: Vec::new(),
                            awaiting_first_prompt: false,
                        };
                        Some(ActivateRoute::SpawnNew)
                    }
                }
            }
        }
    }

    /// `/new`: unconditionally (re)place a Spawning placeholder — unless a
    /// spawn is already in flight, in which case keep the existing
    /// placeholder (and its pending queue) and report it. A Dormant mapping
    /// is replaced like an Active one: `/new` always means a FRESH session,
    /// never a resume.
    pub async fn begin_spawn(&self, key: ChannelKey) -> Result<BeginSpawn, DispatchError> {
        let mut g = self.inner.write().await;
        match g.get(&key) {
            Some(m) if matches!(m.state, MappingState::Spawning { .. }) => {
                Ok(BeginSpawn::AlreadySpawning)
            }
            Some(_) => {
                // A fresh session replaces the active one: queued turns from
                // the old session must not drain into the new one.
                self.clear_queue(&key).await;
                g.insert(key, Mapping::spawning());
                Ok(BeginSpawn::ReplacedActive)
            }
            None => {
                if g.len() >= self.capacity {
                    return Err(DispatchError::Capacity(self.capacity));
                }
                g.insert(key, Mapping::spawning());
                Ok(BeginSpawn::Fresh)
            }
        }
    }

    /// [`SessionMap::begin_spawn`] plus a requested kind/model to remember on
    /// the placeholder (0-turn sessions: the first message spawns the right
    /// agent — P2 fix). `mode`（add-agent-mode-selection）同 model：占位
    /// 记住、首条消息 spawn 时消费，并记为 desired mode 供快照暴露。
    /// `awaiting_first_prompt`（D1）由调用方按占位语义显式给出：0-turn 占位
    /// 传 true；prompt 已直达的真实 spawn 传 false（记录字段但不认领占位
    /// 身份——spawn 窗口内后续消息照常排队，dump 照常过滤 in-flight）。
    /// Only the `Fresh`/`ReplacedActive` insert carries the new fields; an
    /// already-spawning placeholder keeps its existing ones.
    pub async fn begin_spawn_with(
        &self,
        key: ChannelKey,
        kind: Option<String>,
        model: Option<String>,
        mode: Option<String>,
        awaiting_first_prompt: bool,
    ) -> Result<BeginSpawn, DispatchError> {
        let outcome = {
            let mut g = self.inner.write().await;
            match g.get(&key) {
                Some(m) if matches!(m.state, MappingState::Spawning { .. }) => {
                    Ok(BeginSpawn::AlreadySpawning)
                }
                Some(_) => {
                    self.clear_queue(&key).await;
                    g.insert(
                        key.clone(),
                        Mapping::spawning_with(kind, model, mode, awaiting_first_prompt),
                    );
                    Ok(BeginSpawn::ReplacedActive)
                }
                None => {
                    if g.len() >= self.capacity {
                        return Err(DispatchError::Capacity(self.capacity));
                    }
                    g.insert(
                        key.clone(),
                        Mapping::spawning_with(kind, model, mode, awaiting_first_prompt),
                    );
                    Ok(BeginSpawn::Fresh)
                }
            }
        };
        // 0-turn 占位随创建落库（会话创建事件，persist-session-map 2.2）；
        // in-flight 占位非持久形态（persist_upsert 内跳过）——被替换的旧
        // 已提交行原样保留，activate 成功时以新会话行覆盖（crash 中间态
        // 恢复出的是替换前的映射）。
        if matches!(
            outcome,
            Ok(BeginSpawn::Fresh) | Ok(BeginSpawn::ReplacedActive)
        ) {
            self.persist_upsert(&key).await;
        }
        outcome
    }

    /// （add-agent-mode-selection）记录操作者期望的 mode（中途切换）。与
    /// `set_current_model` 同一模式：仅改映射，publish 由 engine 层调用方
    /// 完成。（3.2，D5b）desired 非空：切换必须给出四个控制面词之一。
    pub async fn set_desired_mode(&self, key: &ChannelKey, mode: SessionMode) {
        {
            let mut g = self.inner.write().await;
            if let Some(m) = g.get_mut(key) {
                m.desired_mode = mode;
            } else {
                return;
            }
        }
        // 模式变更即落库（persist-session-map 2.2）。
        self.persist_upsert(key).await;
    }

    /// （add-agent-mode-selection）记录执行体回报的实际生效 mode（本机 =
    /// spawn argv 应用值 / `ModeChanged`；远端 = 节点回报）。
    pub async fn set_effective_mode(&self, key: &ChannelKey, mode: Option<SessionMode>) {
        let mut g = self.inner.write().await;
        if let Some(m) = g.get_mut(key) {
            m.effective_mode = mode;
        }
        // 不落库：effective_mode 是执行体回报的运行时事实，不在持久形状里
        // （重启后如实呈现 None，由下一次回报重建）。
    }

    /// Flip Spawning -> Active and drain queued prompts (returned in order).
    /// Also touches `last_active_unix`: activation is the moment the session
    /// starts doing real work. `acp_session_id` is the agent's real ACP
    /// session id (native-ACP agents) to persist alongside the routing id;
    /// `None` for Claude and whenever the driver reported none.
    ///
    /// `model`（add-acp-model-selection）：spawn outcome 里的模型选择面，写入
    /// 映射供快照暴露（current_model + available_models）；`None` = agent 无
    /// 模型选项。
    pub async fn activate(
        &self,
        key: &ChannelKey,
        session_id: String,
        acp_session_id: Option<String>,
        model: Option<sebas_acp::AcpModelInfo>,
    ) -> Vec<String> {
        let drained = {
            let mut g = self.inner.write().await;
            match g.get_mut(key) {
                Some(m) => {
                    m.last_active_unix = crate::engine::now_unix();
                    let mut next = MappingState::Active { session_id };
                    // 写入映射：新会话/成功 resume 的 acp_session_id 落盘给后续
                    // resume 使用；把旧字段就地一起换掉（D4：load 失败时旧映射
                    // 保持不动——本次 activate 携带的是新会话的 id）。
                    std::mem::swap(&mut m.state, &mut next);
                    let staged: Vec<StagedSubmission> = match next {
                        MappingState::Spawning { pending, .. } => pending,
                        MappingState::Active { .. }
                        | MappingState::Dormant { .. }
                        | MappingState::SpawnFailed { .. } => Vec::new(),
                    };
                    // 合并即投递：这些 id 从此对 remove/move 类型化拒绝为
                    // AlreadyStarted（D2/D7）。delivered 是叶子锁，inner 写锁内
                    // 写入符合既定锁序。
                    {
                        let mut delivered = self.delivered.lock().unwrap();
                        delivered
                            .entry(key.clone())
                            .or_default()
                            .extend(staged.iter().map(|s| s.id));
                    }
                    m.acp_session_id = acp_session_id;
                    m.current_model = model.as_ref().map(|info| info.current.clone());
                    m.available_models = model.map(|info| info.options);
                    staged.into_iter().map(|s| s.text).collect()
                }
                None => {
                    tracing::warn!(
                        ?key,
                        "activate without placeholder; inserting fresh mapping"
                    );
                    let mut mapping = Mapping::active_with_acp(session_id, acp_session_id);
                    mapping.current_model = model.as_ref().map(|info| info.current.clone());
                    mapping.available_models = model.map(|info| info.options);
                    g.insert(key.clone(), mapping);
                    Vec::new()
                }
            }
        };
        // 会话创建/恢复成功即落库（persist-session-map 2.2）：Active 行以同
        // 一会话键 upsert——替换 spawn 窗口前保留的旧已提交行。
        self.persist_upsert(key).await;
        drained
    }

    /// Spawn failed（fail-fast-on-startup-errors 3.1）：Spawning 占位不再被
    /// 静默拆除，而是转为 `SpawnFailed` 终态——返回新映射的 transcript 合成
    /// id，调用方在其下追加 inline 错误条目。只触碰 Spawning 占位——永不动
    /// Active 会话；非 Spawning（含已失败/未知 key）返回 `None` 且无副作用。
    pub async fn fail_spawn(&self, key: &ChannelKey, reason: &str) -> Option<String> {
        let mut g = self.inner.write().await;
        let is_spawning = matches!(
            g.get(key).map(|m| &m.state),
            Some(MappingState::Spawning { .. })
        );
        if !is_spawning {
            return None;
        }
        // 就地翻状态，**不重建映射**：会话身份（project_dir / pending_kind /
        // pending_model / pending_mode / desired_mode …）在 spawn 之前就已记账，
        // 失败不改变「它属于哪个项目、由哪个 agent 服务」。此前整体替换会把这些
        // 一并抹掉——会话因此从项目里消失、agent 展示名回退通用标签，等于把
        // 「启动失败」谎报成「这个会话没有归属」。
        let Some(m) = g.get_mut(key) else { return None };
        let session_id = next_failed_id();
        m.state = MappingState::SpawnFailed {
            session_id: session_id.clone(),
            reason: reason.to_string(),
        };
        m.last_active_unix = crate::engine::now_unix();
        Some(session_id)
    }

    pub async fn insert(&self, key: ChannelKey, mapping: Mapping) -> Result<(), DispatchError> {
        {
            let mut g = self.inner.write().await;
            if !g.contains_key(&key) && g.len() >= self.capacity {
                return Err(DispatchError::Capacity(self.capacity));
            }
            g.insert(key.clone(), mapping);
        }
        // 归档重建等路径的整映射插入同样按变更落库。
        self.persist_upsert(&key).await;
        Ok(())
    }

    /// Set the project_dir on an existing mapping. Used by WebUI to record
    /// the working directory after spawning a project session.
    pub async fn set_project_dir(&self, key: &ChannelKey, project_dir: Option<String>) {
        {
            let mut g = self.inner.write().await;
            if let Some(m) = g.get_mut(key) {
                m.project_dir = project_dir;
            } else {
                return;
            }
        }
        // project_dir 是非飞书会话的从属不变量（0-turn 占位的 spawn 目标）
        // ——置定即落库；这也是占位行通常的首次落库点（创建时无项目不落）。
        self.persist_upsert(key).await;
    }

    pub async fn get(&self, key: &ChannelKey) -> Option<Mapping> {
        self.inner.read().await.get(key).cloned()
    }

    /// The persisted real ACP session id for `key` (native-ACP agents), or
    /// `None` when the mapping is absent or has no recorded id (Claude,
    /// legacy records, Spawning placeholders). Used by the resume path to
    /// load the conversation by the id the agent actually knows instead of
    /// the routing uuid (acp-session-mapping 场景 2).
    pub async fn acp_session_id_for(&self, key: &ChannelKey) -> Option<String> {
        self.inner.read().await.get(key)?.acp_session_id.clone()
    }

    /// 更新会话的 current model 记录（add-acp-model-selection）：SetModel
    /// 成功（ModelChanged）后调用，快照 API 立即反映新模型。无映射时 no-op。
    /// 不更新 available_models（模型列表来自 spawn 时的 configOptions）。
    pub async fn set_current_model(&self, key: &ChannelKey, model_id: String) {
        {
            let mut g = self.inner.write().await;
            if let Some(m) = g.get_mut(key) {
                m.current_model = Some(model_id);
            } else {
                return;
            }
        }
        // 模型变更即落库（persist-session-map 2.2）。
        self.persist_upsert(key).await;
    }

    /// 设置/清空会话 label（fix-webui-approval-restore-and-session-identity
    /// 5.1，design D6）：`None` = 清空（命名回退首条 prompt 预览 / 短 id）。
    /// 任何映射状态都可命名——0-turn 占位（无 session_id）同样可以（label 是
    /// 操作者自由输入）。无映射时返回 `false`（调用方转 UnknownSession）。
    pub async fn set_label(&self, key: &ChannelKey, label: Option<String>) -> bool {
        let hit = {
            let mut g = self.inner.write().await;
            match g.get_mut(key) {
                Some(m) => {
                    m.label = label;
                    true
                }
                None => false,
            }
        };
        // label 变更即落库（persist-session-map 2.2）；无映射时无事可做。
        if hit {
            self.persist_upsert(key).await;
        }
        hit
    }

    /// 自动标题写入（add-agent-settings-and-session-titles 6.2）：**仅当
    /// label 当前为空**时落（操作员已改名 → 标题被丢弃，消除「晚到的标题
    /// 覆盖操作员命名」竞态）。检查与写入在同一把映射锁内，原子。返回是否
    /// 写入；无映射同样 no-op。
    pub async fn set_label_if_empty(&self, key: &ChannelKey, label: String) -> bool {
        let hit = {
            let mut g = self.inner.write().await;
            match g.get_mut(key) {
                Some(m)
                    if m.label
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .is_none() =>
                {
                    m.label = Some(label);
                    true
                }
                _ => false,
            }
        };
        if hit {
            self.persist_upsert(key).await;
        }
        hit
    }

    /// （session-slash-commands 2.1）物化 agent 广告的会话命令表：
    /// `AcpEvent::AvailableCommands` 到达时全量覆盖（二次通知 = 刷新旧表，
    /// 与 model/mode 同一到达线）。无映射时 no-op。空表同样写入——agent 撤
    /// 回广告也是事实，快照如实呈现。
    pub async fn set_available_commands(
        &self,
        key: &ChannelKey,
        commands: Vec<sebas_acp::AvailableCommand>,
    ) {
        let mut g = self.inner.write().await;
        if let Some(m) = g.get_mut(key) {
            m.available_commands = commands;
        }
    }

    /// Preserve a (routing id ↔ real ACP session id) mapping as a dormant
    /// record so a conversation is not lost when a resume falls back fresh
    /// (acp-session-mapping D4: "原映射保留在存储，旧会话仍可被未来 load
    /// 寻址，不因一次失败而抹除"). The record is parked under a deterministic
    /// synthesized `closed-<hash(session_id)>` key so it survives a daemon
    /// restart in the state store yet stays out of the user's chat keys (a
    /// `closed-*` chat can never collide with a web/feishu key, and the WebUI
    /// session list already renders Dormant rows). Idempotent: the same
    /// session id reuses the same archive key instead of duplicating rows.
    ///
    /// `source` 是原映射的键：归档记录**连身份一起保留**（project_dir / agent
    /// kind / mode 等）——「会话必须从属于项目」的不变量下，归档记录若是无项目
    /// 的孤儿就会被持久化层清退，等于把原映射抹掉，正是 D4 要防的事。
    pub async fn preserve_closed_mapping(
        &self,
        source: &ChannelKey,
        session_id: &str,
        acp_session_id: Option<String>,
    ) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        session_id.hash(&mut h);
        let archive_key = ChannelKey::new("web", format!("closed-{:016x}", h.finish()));
        {
            let mut g = self.inner.write().await;
            // 已存在则只补 last_active——不覆盖已记录的 acp_session_id。
            match g.get_mut(&archive_key) {
                Some(m) => {
                    m.last_active_unix = crate::engine::now_unix();
                }
                None => {
                    let mut m =
                        Mapping::dormant(session_id.to_string(), crate::engine::now_unix());
                    m.acp_session_id = acp_session_id;
                    // 身份随记录一起留存（源映射可能已经被替换/移除，取到就带上）。
                    if let Some(src) = g.get(source) {
                        m.project_dir = src.project_dir.clone();
                        m.pending_kind = src.pending_kind.clone();
                        m.pending_model = src.pending_model.clone();
                        m.pending_mode = src.pending_mode.clone();
                        m.desired_mode = src.desired_mode.clone();
                        m.current_model = src.current_model.clone();
                    }
                    if g.len() < self.capacity {
                        g.insert(archive_key.clone(), m);
                    } else {
                        tracing::warn!(%session_id, "session map at capacity; cannot archive closed mapping");
                        return;
                    }
                }
            }
        }
        // 归档记录同样按变更落库（acp-session-mapping D4「原映射保留在存储」
        // 的存储从此就是状态库本身）。
        self.persist_upsert(&archive_key).await;
    }

    pub async fn lookup_key_by_session(&self, session_id: &str) -> Option<ChannelKey> {
        self.inner
            .read()
            .await
            .iter()
            .find(|(_, m)| m.session_id() == Some(session_id))
            .map(|(k, _)| k.clone())
    }

    pub async fn remove_by_session(&self, session_id: &str) {
        let removed = {
            let mut g = self.inner.write().await;
            let found = g
                .iter()
                .find(|(_, m)| m.session_id() == Some(session_id))
                .map(|(k, _)| k.clone());
            if let Some(k) = &found {
                g.remove(k);
            }
            found
        };
        if let Some(k) = removed {
            // 映射被移除（非退役）：持久行一并删除——重启后不得复活。
            self.persist_remove(&k).await;
            // Session torn down: drop queued turns so they never drain into a
            // future session for the same chat key.
            self.clear_queue(&k).await;
        }
    }

    /// （fix-webui-qa-defects-round4 1.2/1.3）terminal teardown 的退役半边：
    /// 活跃绑定（Active）转 **Dormant 记录**，会话行与转录保留——
    /// session-lifecycle delta：teardown 清的是 live binding 与运行时状态，
    /// 「列表消失 / 详情不可达」不属于 teardown。退役后的行以 dormant 态留在
    /// 列表，转录经 `transcript_id`（同 sid）照常可读；下一消息走既有
    /// Dormant→resume 路径，load 被拒时既有诚实回退孵化全新会话——
    /// 「下一消息 fresh spawn」的可观察语义不变。
    ///
    /// 同时：turn 队列与 delivered 集清空（与 remove_by_session 同款——陈旧
    /// prompt 绝不流入未来会话；PendingDropped 的上报由调用方在退役前完成）；
    /// `prompt_preview`（命名来源）在缺省时迁移——卡态即将被丢弃，行名不得
    /// 退化为短 id。幂等：无映射返回 `None`；已 Dormant 只补 last_active 与
    /// 命名。返回退役涉及的 key（`Some` = 发生了 Active→Dormant 转换）。
    pub async fn retire_to_record(
        &self,
        session_id: &str,
        prompt_preview: Option<String>,
    ) -> Option<ChannelKey> {
        let retired = {
            let mut g = self.inner.write().await;
            let retired = if let Some((k, m)) = g
                .iter_mut()
                .find(|(_, m)| m.session_id() == Some(session_id))
            {
                if m.prompt_preview.is_none()
                    && prompt_preview.as_deref().is_some_and(|p| !p.is_empty())
                {
                    m.prompt_preview = prompt_preview;
                }
                m.last_active_unix = crate::engine::now_unix();
                if matches!(m.state, MappingState::Active { .. }) {
                    if let MappingState::Active { session_id: sid } = &m.state {
                        m.state = MappingState::Dormant {
                            session_id: sid.clone(),
                        };
                    }
                    Some(k.clone())
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(k) = &retired {
                // 与 remove_by_session 同款清理：队列与投递记录随活跃绑定消失。
                self.turn_queue.write().await.remove(k);
                self.delivered.lock().unwrap().remove(k);
            }
            retired
        };
        if let Some(k) = &retired {
            // 退役即落库（persist-session-map 2.2）：Active→Dormant 的记录
            // 以 Dormant 行覆盖持久层，重启后按记录保留。
            self.persist_upsert(k).await;
        }
        retired
    }

    /// Remove the mapping for a specific `ChannelKey`, regardless of state.
    /// Used by the WebUI close path to drop Spawning placeholders that have
    /// no session_id (so `remove_by_session` cannot find them).
    pub async fn remove_by_key(&self, key: &ChannelKey) {
        let removed = {
            let mut g = self.inner.write().await;
            g.remove(key).is_some()
        };
        if removed {
            // 关闭即删行（persist-session-map 2.2）——重启后不得复活。
            self.persist_remove(key).await;
            self.clear_queue(key).await;
        }
    }

    /// Drop any queued turns for a session key. Called when a session is torn
    /// down or replaced so stale prompts never drain into a future session.
    /// Also clears the delivered-id set: after teardown the key has no
    /// pending history, so a stale id reads as Unknown rather than
    /// AlreadyStarted (D2 — ids only mean something while the session lives).
    pub async fn clear_queue(&self, key: &ChannelKey) {
        self.turn_queue.write().await.remove(key);
        self.delivered.lock().unwrap().remove(key);
    }

    /// Enqueue a turn for the given session. Priority turns are inserted at
    /// the front; non-priority turns are appended to the back. A pending id
    /// is assigned from the per-key monotonic counter (D2) and returned.
    pub async fn enqueue_turn(&self, key: &ChannelKey, mut turn: QueuedTurn) -> u64 {
        turn.id = self.alloc_id(key);
        let id = turn.id;
        let mut q = self.turn_queue.write().await;
        let deque = q.entry(key.clone()).or_insert_with(VecDeque::new);
        if turn.priority {
            deque.push_front(turn);
        } else {
            deque.push_back(turn);
        }
        id
    }

    /// Pop the next turn from the queue, if any. The popped id is recorded
    /// as delivered (turn_queue 写锁内写入叶子 delivered 锁，D7）so a racing
    /// remove/move deterministically reports AlreadyStarted instead of
    /// Unknown.
    pub async fn pop_next_turn(&self, key: &ChannelKey) -> Option<QueuedTurn> {
        let mut q = self.turn_queue.write().await;
        let popped = q.get_mut(key).and_then(|deque| deque.pop_front());
        if let Some(deque) = q.get(key)
            && deque.is_empty()
        {
            q.remove(key);
        }
        if let Some(turn) = &popped {
            self.delivered
                .lock()
                .unwrap()
                .entry(key.clone())
                .or_default()
                .insert(turn.id);
        }
        popped
    }

    /// Return the number of queued turns for the given session.
    pub async fn queue_len(&self, key: &ChannelKey) -> usize {
        let q = self.turn_queue.read().await;
        q.get(key).map(|deque| deque.len()).unwrap_or(0)
    }

    /// pending submission 观察视图（D1/D6）：staging（投递序在前）+ turn
    /// 队列，`position` 为拼接后的投递序下标。两把锁顺序短暂持有（不嵌套），
    /// 视图是建议性快照——客户端以事件流对账。
    pub async fn pending_submissions(&self, key: &ChannelKey) -> Vec<PendingSubmission> {
        let staged: Vec<PendingSubmission> = {
            let g = self.inner.read().await;
            match g.get(key).map(|m| &m.state) {
                Some(MappingState::Spawning { pending, .. }) => pending
                    .iter()
                    .map(|s| PendingSubmission {
                        id: s.id,
                        text: s.text.clone(),
                        position: 0,
                        disposition: PendingDisposition::Staging,
                        priority: false,
                    })
                    .collect(),
                _ => Vec::new(),
            }
        };
        let turns: Vec<QueuedTurn> = {
            let q = self.turn_queue.read().await;
            q.get(key)
                .map(|d| d.iter().cloned().collect())
                .unwrap_or_default()
        };
        let mut out = staged;
        for turn in turns {
            out.push(PendingSubmission {
                id: turn.id,
                text: turn.prompt,
                position: out.len(),
                disposition: PendingDisposition::Turn,
                priority: turn.priority,
            });
        }
        // position 重算为投递序下标（staging 在前）。
        for (i, p) in out.iter_mut().enumerate() {
            p.position = i;
        }
        out
    }

    /// Remove a pending submission by id (D7). 判定与变更分两段完成，各自
    /// 在对应写锁内原子：staging 段在 inner 写锁（与 activate 互斥），
    /// turn 段在 turn_queue 写锁（与 pop_next_turn 互斥）。已投递的 id 经
    /// delivered 集合确定性拒绝为 AlreadyStarted，绝不回滚在跑的回合。
    pub async fn remove_pending(&self, key: &ChannelKey, id: u64) -> Result<(), PendingOpError> {
        {
            let mut g = self.inner.write().await;
            if let Some(m) = g.get_mut(key)
                && let MappingState::Spawning { pending, .. } = &mut m.state
                && let Some(pos) = pending.iter().position(|s| s.id == id)
            {
                pending.remove(pos);
                return Ok(());
            }
        }
        {
            let mut q = self.turn_queue.write().await;
            if let Some(deque) = q.get_mut(key)
                && let Some(pos) = deque.iter().position(|t| t.id == id)
            {
                deque.remove(pos);
                if deque.is_empty() {
                    q.remove(key);
                }
                return Ok(());
            }
            // 不在队列：查投递记录（pop 在同一把锁内维护，判定无窗口）。
            let delivered = self.delivered.lock().unwrap();
            if delivered.get(key).is_some_and(|set| set.contains(&id)) {
                return Err(PendingOpError::AlreadyStarted);
            }
        }
        Err(PendingOpError::Unknown)
    }

    /// Reorder a pending submission to `to_index` **within its own disposition
    /// group** (D7). `to_index` 以该组当前长度为界（越界 = OutOfRange）；
    /// turn 组维持「优先项构成前缀」不变量——移动会产生优先项落后于普通项
    /// （或普通项插到优先项之前）时，类型化拒绝 PriorityConflict 且不动。
    pub async fn move_pending(
        &self,
        key: &ChannelKey,
        id: u64,
        to_index: usize,
    ) -> Result<(), PendingOpError> {
        {
            let mut g = self.inner.write().await;
            if let Some(m) = g.get_mut(key)
                && let MappingState::Spawning { pending, .. } = &mut m.state
                && let Some(pos) = pending.iter().position(|s| s.id == id)
            {
                if to_index >= pending.len() {
                    return Err(PendingOpError::OutOfRange);
                }
                let item = pending.remove(pos);
                // 缩短后的列表上直接以 to_index 为插入点（夹到尾部）。
                let t = to_index.min(pending.len());
                pending.insert(t, item);
                return Ok(());
            }
        }
        {
            let mut q = self.turn_queue.write().await;
            if let Some(deque) = q.get_mut(key)
                && let Some(pos) = deque.iter().position(|t| t.id == id)
            {
                if to_index >= deque.len() {
                    return Err(PendingOpError::OutOfRange);
                }
                let priorities_before = deque.iter().filter(|t| t.priority).count();
                let item = deque.remove(pos).expect("position checked above");
                // 缩短后的列表上直接以 to_index 为插入点（夹到尾部）。
                let t = to_index.min(deque.len());
                // 不变量：优先项恒为前缀。违反即 PriorityConflict，队列已弹出
                // 的条目放回原位（remove+insert 对同一 deque 的组合在写锁内
                // 完成，外界看不到中间态）。
                let remaining_priorities = if item.priority {
                    priorities_before - 1
                } else {
                    priorities_before
                };
                let ok = if item.priority {
                    t <= remaining_priorities
                } else {
                    t >= remaining_priorities
                };
                if !ok {
                    deque.insert(pos, item);
                    return Err(PendingOpError::PriorityConflict);
                }
                deque.insert(t, item);
                return Ok(());
            }
            let delivered = self.delivered.lock().unwrap();
            if delivered.get(key).is_some_and(|set| set.contains(&id)) {
                return Err(PendingOpError::AlreadyStarted);
            }
        }
        Err(PendingOpError::Unknown)
    }

    /// Return a snapshot of all current mappings. Used by the WebUI to render
    /// the session list. Returns a `Vec<(ChannelKey, Mapping)>` so callers
    /// can iterate without holding the lock.
    pub async fn snapshot_all(&self) -> Vec<(ChannelKey, Mapping)> {
        let g = self.inner.read().await;
        g.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    // ---- 会话映射持久化（persist-session-map）：按变更落盘 ----
    //
    // 映射写入状态库（projects.db 的 session_map 表，ActiveRecord 行 =
    // [`SessionMapRow`]）。生命周期事件处一次 upsert（[SessionMap::persist_upsert]
    // 经单写 actor 串行提交）或删除（[SessionMap::persist_remove]）；**不再有
    // 关停快照**——SIGKILL 只丢未提交的那一笔（openspec/specs/session-lifecycle
    // 的 per-mutation 持久性）。
    //
    // 触发面（2.2）：创建（0-turn 占位与 web_spawn 记账）、activate（spawn/
    // resume 成功）、模型/模式/label/project_dir 变更、归档记录、退役
    // （retire_to_record）与移除（remove_by_session / remove_by_key）。
    // in-flight spawn 占位与 SpawnFailed 终态**不写**（非持久形态：state-store
    // spec「Spawn placeholders are never written」）——既有已提交行原样保留，
    // crash 后恢复的是上一次已提交的映射。

    /// 持久化 `key` 当前的映射（若处于可持久形态）。经全局 state store
    /// 引擎提交；引擎未初始化（单测夹具）时 no-op。写失败只告警——内存
    /// 映射不受影响，重启按上一次已提交状态恢复。
    async fn persist_upsert(&self, key: &ChannelKey) {
        let Some(engine) = crate::state_store::engine() else {
            return;
        };
        // 行的组装在短读锁内完成，绝不跨 await 持锁（叶子锁纪律）。
        let row = {
            let g = self.inner.read().await;
            g.get(key).and_then(|m| session_map_row(key, m))
        };
        let Some(row) = row else { return };
        if let Err(e) = engine.save_session_entry(row).await {
            tracing::warn!(
                error = %e,
                "session map 落库失败（内存映射不受影响，重启按上次已提交状态恢复）"
            );
        }
    }

    /// 从状态库删除 `key` 的映射行（映射被显式移除时）。
    async fn persist_remove(&self, key: &ChannelKey) {
        let Some(engine) = crate::state_store::engine() else {
            return;
        };
        if let Err(e) = engine
            .delete_session_entry(key.channel_str().to_string(), Some(key.reference.clone()))
            .await
        {
            tracing::warn!(error = %e, "session map 行删除失败");
        }
    }

    /// 从状态库行恢复（core 启动；persist-session-map 2.1）。空表 = 空表
    /// 启动；不可寻址的行（缺会话键）按「映射条目不可读」如实告警并跳过
    /// ——**绝不因会话映射而拒绝启动**。每个条目一律回到 `Dormant`（或
    /// 0-turn 占位）：子进程已随上一个 daemon 死亡，映射只用于惰性 respawn。
    pub fn restore_rows(rows: Vec<SessionMapRow>, capacity: usize) -> Self {
        let mut map = HashMap::with_capacity(rows.len());
        for row in rows {
            if let Some((key, m)) = mapping_from_row(row) {
                map.insert(key, m);
            }
        }
        Self {
            inner: Arc::new(RwLock::new(map)),
            turn_queue: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(std::sync::Mutex::new(HashMap::new())),
            delivered: Arc::new(std::sync::Mutex::new(HashMap::new())),
            capacity,
        }
    }
}

/// 「会话必须从属于项目」的唯一判据（写库 / 恢复双向共用）：飞书会话是
/// 例外——它们由聊天发起，本就没有项目目录，只在飞书面呈现；其余通道
/// （web 等）必须带非空 `project_dir`。
fn mapping_may_lack_project(channel: &str, project_dir: Option<&str>) -> bool {
    if channel == "feishu" {
        return true;
    }
    !project_dir.map(str::trim).unwrap_or("").is_empty()
}

/// 一个映射 + 它的会话键 → 状态库行（persist-session-map D2：一行即一个
/// 实例）。非持久形态（in-flight spawn 占位、SpawnFailed）返回 `None`——
/// 调用方跳过写库，已提交行原样保留；违反「会话必须从属于项目」的非飞书
/// 映射同样跳过（debug 留痕——常规流程里占位先创建、project_dir 稍后置定，
/// 此时的跳过是时序事实；真正异常的无项目行由恢复侧 warn 兜底）。
pub fn session_map_row(key: &ChannelKey, m: &Mapping) -> Option<SessionMapRow> {
    let is_placeholder = matches!(
        &m.state,
        MappingState::Spawning {
            awaiting_first_prompt: true,
            ..
        }
    );
    // Active/Dormant 用各自的 id；0-turn 占位落空串占位行。
    let session_id = match m.persisted_id() {
        Some(sid) => sid.to_string(),
        None if is_placeholder => String::new(),
        None => return None,
    };
    if !is_internal_archive_key(key.reference.as_str())
        && !mapping_may_lack_project(key.channel_str(), m.project_dir.as_deref())
    {
        tracing::debug!(
            channel = %key.channel_str(),
            reference = %key.reference,
            "persist: 跳过无项目归属的会话（非飞书通道必须属于项目）"
        );
        return None;
    }
    Some(SessionMapRow {
        chat_id: key.channel_str().to_string(),
        thread_id: Some(key.reference.clone()),
        session_id,
        last_active_unix: m.last_active_unix,
        project_dir: m.project_dir.clone(),
        acp_session_id: m.acp_session_id.clone(),
        current_model: m.current_model.clone(),
        pending_kind: m.pending_kind.clone(),
        pending_model: m.pending_model.clone(),
        pending_mode: m.pending_mode.clone(),
        desired_mode: m.desired_mode.as_str().to_string(),
        label: m.label.clone(),
        prompt_preview: m.prompt_preview.clone(),
        awaiting_first_prompt: is_placeholder,
    })
}

/// 一条状态库行 → (会话键, 恢复后的映射)。不可寻址的行（缺会话键 /
/// 违反项目从属不变量）返回 `None` 并告警——恢复侧跳过，绝不阻塞启动。
pub fn mapping_from_row(row: SessionMapRow) -> Option<(ChannelKey, Mapping)> {
    let reference = match row.thread_id.as_deref() {
        Some(r) if !r.is_empty() => r.to_string(),
        _ => {
            tracing::warn!(
                chat_id = %row.chat_id,
                "session_map 行缺会话键（thread_id 空），跳过恢复"
            );
            return None;
        }
    };
    let key = ChannelKey::new(row.chat_id.as_str(), reference);
    if !is_internal_archive_key(key.reference.as_str())
        && !mapping_may_lack_project(key.channel_str(), row.project_dir.as_deref())
    {
        tracing::warn!(
            channel = %key.channel_str(),
            reference = %key.reference,
            "restore: 丢弃无项目归属的会话（非飞书通道必须属于项目）"
        );
        return None;
    }
    let m = if row.awaiting_first_prompt && row.session_id.is_empty() {
        // 0-turn 占位（workbench-agent-wire-fix D1）：重启后仍是等待首条
        // 消息的占位，首条消息照常触发 spawn。
        let mut m = Mapping::spawning_with(
            row.pending_kind.clone(),
            row.pending_model.clone(),
            row.pending_mode.clone(),
            true,
        );
        m.desired_mode = SessionMode::from_wire(&row.desired_mode);
        m.project_dir = row.project_dir.clone();
        m.label = row.label.clone();
        m.prompt_preview = row.prompt_preview.clone();
        m
    } else {
        let mut m = Mapping::dormant(row.session_id, row.last_active_unix);
        m.acp_session_id = row.acp_session_id;
        m.current_model = row.current_model;
        m.pending_kind = row.pending_kind;
        m.pending_model = row.pending_model;
        m.pending_mode = row.pending_mode;
        m.desired_mode = SessionMode::from_wire(&row.desired_mode);
        m.project_dir = row.project_dir;
        m.label = row.label;
        m.prompt_preview = row.prompt_preview;
        m
    };
    Some((key, m))
}

/// 内部归档记录键（`closed-<hash>`）：不是会话，是「原映射保留在存储」的
/// 存储记录（acp-session-mapping D4），因此不受「会话必须从属于项目」约束。
fn is_internal_archive_key(reference: &str) -> bool {
    reference.starts_with("closed-")
}

impl Default for SessionMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fix-webui-approval-restore-and-session-identity 5.1：label 随映射行
    /// 落库 → 恢复保留；无 label 的行恢复为 None（命名回退不变）。
    #[tokio::test]
    async fn label_round_trips_through_row_shape() {
        let key = ChannelKey::new("web", "web-label");
        let mut m = Mapping::dormant("s-label", 1);
        m.project_dir = Some("/tmp/proj-label".into());
        m.label = Some("重构计划".into());
        let row = session_map_row(&key, &m).expect("persistable");
        assert_eq!(row.label.as_deref(), Some("重构计划"), "label must be persisted");

        let (back_key, back) = mapping_from_row(row).expect("addressable");
        assert_eq!(back_key, key);
        assert_eq!(back.label.as_deref(), Some("重构计划"));

        // 无 label 的行 → None。
        let bare = SessionMapRow {
            chat_id: "web".into(),
            thread_id: Some("web-old".into()),
            session_id: "s-old".into(),
            last_active_unix: 1,
            project_dir: Some("/tmp/p".into()),
            acp_session_id: None,
            current_model: None,
            pending_kind: None,
            pending_model: None,
            pending_mode: None,
            desired_mode: crate::engine::ask_mode().as_str().to_string(),
            label: None,
            prompt_preview: None,
            awaiting_first_prompt: false,
        };
        let (_, old_m) = mapping_from_row(bare).unwrap();
        assert_eq!(old_m.label, None, "bare rows restore without a label");
    }

    /// add-composer-agent-binding 1.2：pending_kind 随映射行落库 → 恢复保留；
    /// 无该字段的行 → None（UI 回退默认 kind 标签）。
    #[tokio::test]
    async fn pending_kind_round_trips_through_row_shape() {
        let key = ChannelKey::new("web", "web-kind");
        let mut m = Mapping::dormant("s1", 1);
        m.pending_kind = Some("claude".into());
        // web 会话必须从属于项目（否则写库按不变量跳过它）。
        m.project_dir = Some("/tmp/proj-kind".into());
        let row = session_map_row(&key, &m).expect("persistable");
        assert_eq!(
            row.pending_kind.as_deref(),
            Some("claude"),
            "row carries pending_kind"
        );
        let (_, restored) = mapping_from_row(row).unwrap();
        assert_eq!(
            restored.pending_kind.as_deref(),
            Some("claude"),
            "restore keeps the bound kind"
        );

        let bare_key = ChannelKey::new("web", "web-bare");
        let mut bare = Mapping::dormant("s2", 2);
        bare.pending_kind = None;
        bare.project_dir = Some("/tmp/proj-bare".into());
        let (_, restored_bare) = mapping_from_row(session_map_row(&bare_key, &bare).unwrap())
            .unwrap();
        assert_eq!(restored_bare.pending_kind, None);
    }

    /// add-agent-settings-and-session-titles 6.2：自动标题只在 label 为空时
    /// 落——操作员先改名则晚到的标题被丢弃（竞态消除）；空白 label 视同
    /// 未命名；无映射 no-op。
    #[tokio::test]
    async fn auto_title_writes_only_while_label_is_empty() {
        let map = SessionMap::new();
        let key = ChannelKey::new("web", "web-auto-title");
        map.insert(
            key.clone(),
            Mapping::dormant("s-auto-title", 1),
        )
        .await
        .expect("fresh key inserts");

        // label 为空 → 落。
        assert!(map.set_label_if_empty(&key, "自动标题".into()).await);
        assert_eq!(
            map.get(&key).await.unwrap().label.as_deref(),
            Some("自动标题")
        );

        // 已有 label → 晚到的标题被丢弃（操作员命名不被覆盖）。
        assert!(!map.set_label_if_empty(&key, "另一个标题".into()).await);
        assert_eq!(
            map.get(&key).await.unwrap().label.as_deref(),
            Some("自动标题"),
            "operator label survives the late title"
        );

        // 操作员清除 label → 机制上允许再写（是否再生由触发面决定：不在
        // 首条消息场景之外触发）。
        map.set_label(&key, None).await;
        assert!(map.set_label_if_empty(&key, "新标题".into()).await);

        // 空白 label 视同未命名。
        map.set_label(&key, Some("   ".into())).await;
        assert!(map.set_label_if_empty(&key, "再写".into()).await);

        // 无映射 no-op。
        let ghost = ChannelKey::new("web", "web-auto-title-ghost");
        assert!(!map.set_label_if_empty(&ghost, "x".into()).await);
    }

    /// 非持久形态（in-flight 占位 / SpawnFailed）不产出行——已提交行原样
    /// 保留到 activate 覆盖（persist-session-map：占位不落盘）。
    #[tokio::test]
    async fn non_persistable_states_yield_no_row() {
        let key = ChannelKey::feishu("oc_np", None);
        // in-flight spawn 占位（awaiting_first_prompt = false）。
        let inflight = Mapping::spawning();
        assert!(session_map_row(&key, &inflight).is_none());
        // SpawnFailed 终态。
        let mut failed = Mapping::dormant("s-f", 1);
        failed.state = MappingState::SpawnFailed {
            session_id: "failed-1".into(),
            reason: "boom".into(),
        };
        assert!(session_map_row(&key, &failed).is_none());
        // Active / Dormant / 0-turn 占位可持久。
        assert!(session_map_row(&key, &Mapping::active("s-a")).is_some());
        assert!(session_map_row(&key, &Mapping::dormant("s-d", 1)).is_some());
        let placeholder =
            Mapping::spawning_with(Some("claude".into()), None, None, true);
        let row = session_map_row(&key, &placeholder).unwrap();
        assert_eq!(row.session_id, "", "占位行落空 session_id");
        assert!(row.awaiting_first_prompt);
    }

    /// 「会话必须从属于项目」：无项目的非飞书映射跳过写库/恢复；归档键
    /// （closed-*）与飞书会话豁免。
    #[tokio::test]
    async fn project_ownership_invariant_gates_rows() {
        let key = ChannelKey::new("web", "web-orphan");
        let m = Mapping::dormant("s1", 1);
        assert!(
            session_map_row(&key, &m).is_none(),
            "无项目的 web 行不得写库"
        );
        // 恢复侧同样丢弃（warn + None）。
        let orphan_row = SessionMapRow {
            chat_id: "web".into(),
            thread_id: Some("web-orphan".into()),
            session_id: "s1".into(),
            last_active_unix: 1,
            project_dir: None,
            acp_session_id: None,
            current_model: None,
            pending_kind: None,
            pending_model: None,
            pending_mode: None,
            desired_mode: crate::engine::ask_mode().as_str().to_string(),
            label: None,
            prompt_preview: None,
            awaiting_first_prompt: false,
        };
        assert!(mapping_from_row(orphan_row).is_none());
        // 飞书豁免。
        let feishu = ChannelKey::feishu("oc_x", None);
        assert!(session_map_row(&feishu, &Mapping::dormant("s2", 2)).is_some());
    }

    /// 不可寻址的行（缺会话键）恢复时被跳过——映射条目不可读 → 空表 +
    /// 告警，绝不阻塞启动（session-lifecycle 恢复要求）。
    #[tokio::test]
    async fn unaddressable_rows_are_skipped_not_fatal() {
        let rows = vec![SessionMapRow {
            chat_id: "web".into(),
            thread_id: None,
            session_id: "s1".into(),
            last_active_unix: 1,
            project_dir: Some("/tmp/p".into()),
            acp_session_id: None,
            current_model: None,
            pending_kind: None,
            pending_model: None,
            pending_mode: None,
            desired_mode: crate::engine::ask_mode().as_str().to_string(),
            label: None,
            prompt_preview: None,
            awaiting_first_prompt: false,
        }];
        let map = SessionMap::restore_rows(rows, 8);
        assert!(
            map.snapshot_all().await.is_empty(),
            "不可寻址行跳过后为空表"
        );
    }

    // ── workbench-turn-queue 3.1：per-key 单调 id、顺序稳定、drain 后失效 ──

    /// 入队分配唯一 id；pending_submissions 顺序稳定（staging 在前、turn
    /// 按序）；activate 合并后 staging 条目离开 pending 栈，pop 后 turn 条目
    /// 离开——id 失效但可被确定性识别为 AlreadyStarted。
    #[tokio::test]
    async fn pending_ids_unique_order_stable_and_void_after_delivery() {
        let map = SessionMap::new();
        let key = ChannelKey::new("web", "web-q");

        // Spawning 占位先行（首条文本会触发 SpawnNew 成为首条 prompt，不进
        // staging），其后的文本才被暂存。
        map.begin_spawn(key.clone()).await.unwrap();
        map.route_text(key.clone(), "staged-1".into())
            .await
            .unwrap();
        map.route_text(key.clone(), "staged-2".into())
            .await
            .unwrap();

        let mut turn_ids = std::collections::HashSet::new();
        for i in 0..3 {
            let id = map
                .enqueue_turn(&key, QueuedTurn::new(format!("turn-{i}"), None, false))
                .await;
            assert!(turn_ids.insert(id), "ids must be unique");
        }
        let mut staged_id = None;
        for p in map.pending_submissions(&key).await {
            match p.disposition {
                PendingDisposition::Staging => staged_id = Some(p.id),
                PendingDisposition::Turn => {}
            }
        }
        let staged_id = staged_id.expect("staged entry visible in the view");

        // 顺序稳定：连读两次，投递序逐项一致。
        let a = map.pending_submissions(&key).await;
        let b = map.pending_submissions(&key).await;
        assert_eq!(a, b, "view order must be stable across reads");
        assert_eq!(a.len(), 5, "2 staged + 3 turns");
        assert_eq!(
            a.iter().map(|p| p.position).collect::<Vec<_>>(),
            (0..5).collect::<Vec<_>>(),
            "positions are the delivery-order index"
        );
        assert_eq!(a[0].disposition, PendingDisposition::Staging);
        assert_eq!(a[1].disposition, PendingDisposition::Staging);
        assert_eq!(a[2].disposition, PendingDisposition::Turn);

        // activate 合并 staged：staged 条目离开栈，id 类型化为已开始。
        let drained = map.activate(&key, "s1".into(), None, None).await;
        assert_eq!(
            drained,
            vec!["staged-1".to_string(), "staged-2".to_string()]
        );
        let view = map.pending_submissions(&key).await;
        assert_eq!(view.len(), 3, "staged entries left the stack at activation");
        assert_eq!(
            map.remove_pending(&key, staged_id).await,
            Err(PendingOpError::AlreadyStarted),
            "combined staged id is deterministically 'already started'"
        );

        // pop 投递 turn：id 失效 + AlreadyStarted。
        let popped = map.pop_next_turn(&key).await.expect("queued turn");
        assert_eq!(popped.prompt, "turn-0");
        assert_eq!(
            map.remove_pending(&key, popped.id).await,
            Err(PendingOpError::AlreadyStarted),
            "popped id reports AlreadyStarted, never Unknown"
        );
        assert_eq!(map.pending_submissions(&key).await.len(), 2);
    }

    // ── workbench-turn-queue 3.2：remove/move 的类型化拒绝与成功路径 ──

    #[tokio::test]
    async fn remove_pending_covers_rejections_and_success() {
        let map = SessionMap::new();
        let key = ChannelKey::new("web", "web-rm");
        map.begin_spawn(key.clone()).await.unwrap();
        map.route_text(key.clone(), "staged".into()).await.unwrap();
        let t1 = map
            .enqueue_turn(&key, QueuedTurn::new("t1", None, false))
            .await;
        let t2 = map
            .enqueue_turn(&key, QueuedTurn::new("t2", None, false))
            .await;

        // 成功：移除 turn 条目。
        assert_eq!(map.remove_pending(&key, t1).await, Ok(()));
        assert!(
            !map.pending_submissions(&key)
                .await
                .iter()
                .any(|p| p.id == t1)
        );
        // 成功：移除 staged 条目。
        let staged_id = map
            .pending_submissions(&key)
            .await
            .iter()
            .find(|p| p.disposition == PendingDisposition::Staging)
            .expect("staged entry")
            .id;
        assert_eq!(map.remove_pending(&key, staged_id).await, Ok(()));
        // Unknown：未分配过的 id。
        assert_eq!(
            map.remove_pending(&key, 99_999).await,
            Err(PendingOpError::Unknown)
        );
        // 已开始（pop 投递）→ AlreadyStarted。
        map.pop_next_turn(&key).await.expect("t2");
        assert_eq!(
            map.remove_pending(&key, t2).await,
            Err(PendingOpError::AlreadyStarted)
        );
        // 用户已删除的 id（未投递）→ Unknown，不误报 AlreadyStarted。
        let t3 = map
            .enqueue_turn(&key, QueuedTurn::new("t3", None, false))
            .await;
        assert_eq!(map.remove_pending(&key, t3).await, Ok(()));
        assert_eq!(
            map.remove_pending(&key, t3).await,
            Err(PendingOpError::Unknown)
        );
    }

    #[tokio::test]
    async fn move_pending_covers_rejections_and_success() {
        let map = SessionMap::new();
        let key = ChannelKey::new("web", "web-mv");
        let p = map
            .enqueue_turn(&key, QueuedTurn::new("p", None, true))
            .await; // priority
        let n1 = map
            .enqueue_turn(&key, QueuedTurn::new("n1", None, false))
            .await;
        let _n2 = map
            .enqueue_turn(&key, QueuedTurn::new("n2", None, false))
            .await;

        // OutOfRange：越界落点。
        assert_eq!(
            map.move_pending(&key, n1, 3).await,
            Err(PendingOpError::OutOfRange)
        );
        // PriorityConflict：普通项插到优先项之前。
        assert_eq!(
            map.move_pending(&key, n1, 0).await,
            Err(PendingOpError::PriorityConflict)
        );
        // 移动后的队列未被破坏。
        let view = map.pending_submissions(&key).await;
        assert_eq!(
            view.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(),
            vec!["p", "n1", "n2"]
        );
        // PriorityConflict：优先项拖到队尾。
        assert_eq!(
            map.move_pending(&key, p, 2).await,
            Err(PendingOpError::PriorityConflict)
        );
        // 成功：普通项在非优先段内后移（n1 → 队尾）。
        assert_eq!(map.move_pending(&key, n1, 2).await, Ok(()));
        assert_eq!(
            map.pending_submissions(&key)
                .await
                .iter()
                .map(|x| x.text.as_str())
                .collect::<Vec<_>>(),
            vec!["p", "n2", "n1"]
        );
        // 成功：普通项前移但不越过优先项（n1 → 优先项后第一位）。
        assert_eq!(map.move_pending(&key, n1, 1).await, Ok(()));
        assert_eq!(
            map.pending_submissions(&key)
                .await
                .iter()
                .map(|x| x.text.as_str())
                .collect::<Vec<_>>(),
            vec!["p", "n1", "n2"]
        );
        // 成功：优先项在优先段内移动（唯一优先项 → 头部，仍是前缀）。
        assert_eq!(map.move_pending(&key, p, 0).await, Ok(()));
        // Unknown。
        assert_eq!(
            map.move_pending(&key, 42, 0).await,
            Err(PendingOpError::Unknown)
        );
    }

    /// staging 组的重排：组内自由移动（无优先概念），越界拒绝。
    #[tokio::test]
    async fn move_pending_within_staging_group() {
        let map = SessionMap::new();
        let key = ChannelKey::new("web", "web-mv-staging");
        map.begin_spawn(key.clone()).await.unwrap();
        map.route_text(key.clone(), "a".into()).await.unwrap();
        map.route_text(key.clone(), "b".into()).await.unwrap();
        map.route_text(key.clone(), "c".into()).await.unwrap();
        let view = map.pending_submissions(&key).await;
        let b_id = view[1].id;
        assert_eq!(
            map.move_pending(&key, b_id, 3).await,
            Err(PendingOpError::OutOfRange)
        );
        assert_eq!(map.move_pending(&key, b_id, 0).await, Ok(()));
        assert_eq!(
            map.pending_submissions(&key)
                .await
                .iter()
                .map(|x| x.text.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );
    }

    // ── workbench-turn-queue 3.3：remove/move 与 activate/drain 的竞态 ──

    /// 并发 remove 与 activate：结果只允许两种——remove 成功（条目不进
    /// drained 文本）或 remove 拿到 AlreadyStarted（activate 先合并）。绝不
    /// 出现「已删除的条目仍被投递」。
    #[tokio::test]
    async fn remove_race_activate_never_delivers_removed_entries() {
        for _ in 0..50 {
            let map = SessionMap::new();
            let key = ChannelKey::new("web", "web-race-act");
            map.begin_spawn(key.clone()).await.unwrap();
            map.route_text(key.clone(), "race-staged".into())
                .await
                .unwrap();
            let id = map.pending_submissions(&key).await[0].id;

            let m2 = map.clone();
            let k2 = key.clone();
            let (remove_res, drained) = tokio::join!(m2.remove_pending(&k2, id), async {
                map.activate(&key, "s1".into(), None, None).await
            });
            match remove_res {
                Ok(()) => assert!(
                    !drained.iter().any(|t| t == "race-staged"),
                    "removed entry must not be delivered at activation"
                ),
                Err(PendingOpError::AlreadyStarted) => {
                    assert_eq!(drained, vec!["race-staged".to_string()])
                }
                Err(other) => panic!("unexpected rejection in the race: {other:?}"),
            }
        }
    }

    /// 并发 remove 与 pop（drain）：remove 成功 → 该 id 不被 pop 投递；
    /// remove AlreadyStarted → pop 拿走的正是它。绝不回滚已投递回合。
    #[tokio::test]
    async fn remove_race_pop_never_delivers_removed_turns() {
        for _ in 0..50 {
            let map = SessionMap::new();
            let key = ChannelKey::new("web", "web-race-pop");
            let target = map
                .enqueue_turn(&key, QueuedTurn::new("target", None, false))
                .await;
            map.enqueue_turn(&key, QueuedTurn::new("tail", None, false))
                .await;

            let m2 = map.clone();
            let k2 = key.clone();
            let (remove_res, popped) =
                tokio::join!(m2.remove_pending(&k2, target), map.pop_next_turn(&key));
            let popped = popped.expect("queue holds two turns during the race");
            match remove_res {
                Ok(()) => assert_ne!(popped.id, target, "a removed turn must never be delivered"),
                Err(PendingOpError::AlreadyStarted) => {}
                Err(other) => panic!("unexpected rejection in the race: {other:?}"),
            }
        }
    }

    /// 已在跑的回合不被回滚：pop 投递后，move/remove 只能拿到
    /// AlreadyStarted，队列与其余条目不受影响。
    #[tokio::test]
    async fn running_turn_is_never_rolled_back_by_management_ops() {
        let map = SessionMap::new();
        let key = ChannelKey::new("web", "web-running");
        let first = map
            .enqueue_turn(&key, QueuedTurn::new("first", None, false))
            .await;
        let _second = map
            .enqueue_turn(&key, QueuedTurn::new("second", None, false))
            .await;
        let popped = map.pop_next_turn(&key).await.expect("pop");
        assert_eq!(popped.id, first);
        assert_eq!(
            map.move_pending(&key, first, 0).await,
            Err(PendingOpError::AlreadyStarted)
        );
        assert_eq!(
            map.remove_pending(&key, first).await,
            Err(PendingOpError::AlreadyStarted)
        );
        // 剩余队列完好。
        assert_eq!(map.queue_len(&key).await, 1);
        let next = map.pop_next_turn(&key).await.expect("second turn intact");
        assert_eq!(next.prompt, "second");
    }

    /// 5.1：staging 队列满 16 条 → 第 17 条 Overflow 拒绝且不分配 id、
    /// 不顶掉已暂存条目。
    #[tokio::test]
    async fn staging_overflow_rejects_visibly_without_displacing() {
        let map = SessionMap::new();
        let key = ChannelKey::new("web", "web-cap");
        map.begin_spawn(key.clone()).await.unwrap();
        for i in 0..MAX_PENDING_SUBMISSIONS {
            assert!(matches!(
                map.route_text(key.clone(), format!("m{i}")).await.unwrap(),
                TextRoute::Enqueued
            ));
        }
        match map.route_text(key.clone(), "over".into()).await.unwrap() {
            TextRoute::Overflow { cap } => assert_eq!(cap, MAX_PENDING_SUBMISSIONS),
            other => panic!("expected Overflow, got {other:?}"),
        }
        let view = map.pending_submissions(&key).await;
        assert_eq!(view.len(), MAX_PENDING_SUBMISSIONS, "nothing displaced");
        assert!(
            !view.iter().any(|p| p.text == "over"),
            "rejected text not staged"
        );
    }
}

#[cfg(test)]
mod desired_mode_migration_tests {
    use super::*;
    use crate::engine::ASK_MODE;

    /// （session-parallel-liveness-and-unread-polish 3.2，D5b）desired_mode
    /// 是非空控制面词：行原样携带、恢复原样读回——盘上不再有 null 形态
    /// （persist-session-map：映射持久化形状即行，无 serde 中间层）。
    #[tokio::test]
    async fn desired_mode_round_trips_verbatim_through_rows() {
        let k = |r: &str| ChannelKey::new("web", r);
        let mut migrated = Mapping::dormant("s1", 1);
        migrated.project_dir = Some("/tmp/p1".into());
        migrated.desired_mode = ASK_MODE.into();
        let mut explicit = Mapping::dormant("s2", 1);
        explicit.project_dir = Some("/tmp/p2".into());
        explicit.desired_mode = "auto".into();

        let rows = vec![
            session_map_row(&k("web-null"), &migrated).unwrap(),
            session_map_row(&k("web-explicit"), &explicit).unwrap(),
        ];
        let map = SessionMap::restore_rows(rows, 8);
        assert_eq!(
            map.get(&k("web-null")).await.unwrap().desired_mode,
            SessionMode::from(ASK_MODE),
            "ask 词原样往返"
        );
        assert_eq!(
            map.get(&k("web-explicit")).await.unwrap().desired_mode,
            SessionMode::Auto,
            "已是合法词的值原样保留"
        );
    }

    /// 内存构造路径同样非空：`Mapping::active`/`spawning`/`dormant` 的
    /// desired_mode 都是缺省 ask，绝不出现空路径。
    #[tokio::test]
    async fn mapping_constructors_default_to_ask() {
        assert_eq!(Mapping::active("s").desired_mode, SessionMode::Ask);
        assert_eq!(Mapping::dormant("s", 0).desired_mode, SessionMode::Ask);
        assert_eq!(Mapping::spawning().desired_mode, SessionMode::Ask);
        // 创建请求未点名 mode 的占位同样落 ask（D5b：0-turn 占位行真源即 ask）。
        assert_eq!(
            Mapping::spawning_with(None, None, None, true).desired_mode,
            SessionMode::Ask
        );
        assert_eq!(
            Mapping::spawning_with(None, None, Some("edit".into()), true).desired_mode,
            SessionMode::Edit
        );
    }
}

// ── close-acceptance-blind-spots 盲区 4：重启 spawning 收敛（design D2 重投优先）──
#[cfg(test)]
mod spawning_settle_tests {
    use super::*;
    use crate::engine::{DispatchHandle, Out};
    use std::time::Duration;

    /// 模拟一份「实例在 spawning 相位退出」留下的状态库行（projects.db 的
    /// session_map 表）：两个 0-turn 占位（恢复后即 spawning 相位会话，一个
    /// 带完整创建参数、一个只有 kind）+ 一个普通 dormant 会话（对照：落定
    /// 不得误伤）。
    fn seeded_state_rows() -> Vec<SessionMapRow> {
        let base = |chat: &str, thread: &str| SessionMapRow {
            chat_id: chat.into(),
            thread_id: Some(thread.into()),
            session_id: String::new(),
            last_active_unix: 0,
            project_dir: None,
            acp_session_id: None,
            current_model: None,
            pending_kind: None,
            pending_model: None,
            pending_mode: None,
            desired_mode: crate::engine::ask_mode().as_str().to_string(),
            label: None,
            prompt_preview: None,
            awaiting_first_prompt: false,
        };
        let mut placeholder = base("web", "web-restore-sp");
        placeholder.last_active_unix = 10;
        placeholder.awaiting_first_prompt = true;
        placeholder.pending_kind = Some("claude".into());
        placeholder.pending_model = Some("sonnet-x".into());
        placeholder.pending_mode = Some("edit".into());
        placeholder.desired_mode = "edit".into();
        placeholder.project_dir = Some("/tmp/proj-settle".into());
        let mut bare = base("web", "web-restore-sp-bare");
        bare.last_active_unix = 11;
        bare.awaiting_first_prompt = true;
        bare.pending_kind = Some("opencode".into());
        bare.project_dir = Some("/tmp/proj-settle".into());
        let mut dormant = base("web", "web-restore-dormant");
        dormant.last_active_unix = 12;
        dormant.session_id = "s-old".into();
        dormant.project_dir = Some("/tmp/proj-settle".into());
        vec![placeholder, bare, dormant]
    }

    async fn first_out(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Out {
        tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .expect("out within 500ms")
            .expect("channel open")
    }

    /// 3.1 落定语义（重投优先）：恢复出的 spawning 占位逐个重投 spawn 指令——
    /// 空 prompt 的 WebSpawn（激活语义）携带创建时记住的 project_dir/kind/
    /// model/mode；占位标记被消费（重新进入 spawn 流程，后续消息走 staging 而
    /// 不是再触发一次 spawn）。dormant 会话原样保留（落定互不阻塞、不误伤）。
    /// 幂等：重复落定不再二次投递（在飞占位走 AlreadyStarting）。
    #[tokio::test]
    async fn restored_spawning_placeholders_redispatch_spawn_instructions() {
        let map = SessionMap::restore_rows(seeded_state_rows(), usize::MAX);
        // 前置事实：恢复面确实有 spawning 相位会话（占位）。
        let sp_key = ChannelKey::new("web", "web-restore-sp");
        assert!(map.get(&sp_key).await.unwrap().awaiting_first_prompt());
        let (router, mut out_rx) = DispatchHandle::new(map);

        let redispatched = router.settle_restored_spawning().await;
        assert_eq!(redispatched, 2, "两个占位各自重投一次");

        // 重投 = 指令重发：空 prompt（激活语义）+ 创建时记住的参数。
        // settle_restored_spawning 遍历 HashMap，投递顺序不定——收齐两条后
        // 按 reference 归位断言，不做顺序假设。
        let mut spawns = Vec::new();
        for _ in 0..2 {
            match first_out(&mut out_rx).await {
                Out::WebSpawn {
                    key,
                    prompt,
                    project_dir,
                    kind,
                    model,
                    mode,
                } => spawns.push((key, prompt, project_dir, kind, model, mode)),
                other => panic!("expected WebSpawn, got {other:?}"),
            }
        }
        spawns.sort_by(|a, b| a.0.reference.cmp(&b.0.reference));
        let (key, prompt, project_dir, kind, model, mode) = &spawns[0];
        assert_eq!(key.reference, "web-restore-sp");
        assert_eq!(key.channel_str(), "web");
        assert_eq!(prompt, "", "重投走激活语义，不带首轮 prompt");
        assert_eq!(project_dir.as_deref(), Some("/tmp/proj-settle"));
        assert_eq!(kind.as_deref(), Some("claude"));
        assert_eq!(model.as_deref(), Some("sonnet-x"));
        assert_eq!(mode.as_deref(), Some("edit"));
        let (bare_key, _, _, bare_kind, bare_model, _) = &spawns[1];
        assert_eq!(bare_key.reference, "web-restore-sp-bare");
        assert_eq!(bare_kind.as_deref(), Some("opencode"));
        assert!(bare_model.is_none(), "未记模型的占位如实不带 model");

        // 占位标记已消费：重新进入 spawn 流程（spec 允许的落定形态），后续
        // 消息在 spawn 窗口内走 staging，不再误判成占位首条消息二次 spawn。
        assert!(
            !router.map.get(&sp_key).await.unwrap().awaiting_first_prompt(),
            "占位标记必须被重投消费"
        );
        // 对照：dormant 会话的恢复不受落定影响（不阻塞、不误伤）。
        let dormant_key = ChannelKey::new("web", "web-restore-dormant");
        let dormant = router.map.get(&dormant_key).await.unwrap();
        assert!(
            matches!(dormant.state, MappingState::Dormant { session_id: ref s } if s == "s-old"),
            "dormant 会话必须原样保留，got {:?}",
            dormant.state
        );

        // 幂等：再次落定无事可做、不二次投递（防重复 spawn）。
        assert_eq!(router.settle_restored_spawning().await, 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), out_rx.recv())
                .await
                .is_err(),
            "幂等重放不得再发 spawn 指令"
        );
    }

    /// 3.1 失败分支：重投后的 spawn 失败（出站泵 handle_web_spawn 的 Err 臂
    /// 同款落点 fail_spawn）→ 会话投影追加合成错误条目 + spawn-failed 非
    /// spawning 终态——绝不留无人收敛的 spawning 僵尸。
    #[tokio::test]
    async fn failed_redispatch_lands_synthetic_error_and_terminal_state() {
        let map = SessionMap::restore_rows(seeded_state_rows(), usize::MAX);
        let (router, mut out_rx) = DispatchHandle::new(map);
        assert_eq!(router.settle_restored_spawning().await, 2);

        let Out::WebSpawn { key, .. } = first_out(&mut out_rx).await else {
            panic!("expected WebSpawn")
        };
        // 泵侧失败臂的既有收敛路径（agent 配置缺失 = unknown agent kind）。
        router.fail_spawn(&key, "unknown agent kind \"ghost\"").await;

        let m = router.map.get(&key).await.unwrap();
        let reason = m
            .spawn_failed_reason()
            .expect("落失败分支必须停在 spawn-failed 终态");
        assert_eq!(reason, "unknown agent kind \"ghost\"");
        assert!(
            matches!(m.state, MappingState::SpawnFailed { .. }),
            "终态非 spawning：{:?}",
            m.state
        );

        // 合成错误条目在会话投影（合成 transcript id 名下）可读。
        let turns = router.session_turns(&key, 0).await.expect("transcript");
        assert_eq!(turns.len(), 1, "合成错误条目恰好一条");
        assert_eq!(turns[0].element_type, sebas_domain::session::TurnElementType::Error);
        assert!(turns[0].content.contains("spawn failed"));

        // 对外状态面不再是 spawning。
        let info = router.session_info_for(&key).await.expect("session info");
        assert_eq!(info.status, SessionPhase::SpawnFailed);
        assert_eq!(info.spawn_failure_reason.as_deref(), Some(reason));
    }
}
