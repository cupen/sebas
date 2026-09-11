use crate::error::DispatchError;
use sebas_channels::ChannelKey;
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
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

/// 处置（workbench-turn-queue D1）：`staging` = 并入首条消息；`turn` = 按序
/// 执行的待执行回合。serde 形状随 `SessionInfo` 上 wire。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingDisposition {
    Staging,
    Turn,
}

/// pending submission 的观察视图（design D1）：core 已接受、尚未开始执行的
/// 一次提交。`position` 是投递序里的下标（staging 先于 turn 队列），每次
/// 构建视图时重算。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingSubmission {
    pub id: u64,
    pub text: String,
    pub position: usize,
    pub disposition: PendingDisposition,
    pub priority: bool,
}

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
/// appears at runtime except via `restore_json`.
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

fn next_failed_id() -> String {
    let n = FAILED_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("failed-{n}")
}

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
    /// （add-agent-mode-selection）操作者期望的会话 mode：创建请求携带、
    /// 中途切换立即更新。快照暴露给前端；`None` = agent 默认。
    pub desired_mode: Option<String>,
    /// （add-agent-mode-selection）执行体回报的**实际生效** mode（本机 =
    /// spawn argv 实际应用值 / `ModeChanged` 事件；远端 = 节点回报）。
    /// `None` = 执行体未声称任何 mode 生效（如实呈现 desired/effective 差异）。
    pub effective_mode: Option<String>,
    /// The agent's real ACP session id when it differs from the routing id
    /// (native-ACP agents, e.g. opencode; the `session/new` id on a fresh
    /// spawn, the loaded conversation id on a successful resume). `None` for
    /// Claude (routing id == conversation id) and legacy records. Persisted
    /// with the session record; a resume reads it to load the conversation
    /// by the id the agent actually knows (acp-session-mapping D3).
    pub acp_session_id: Option<String>,
    /// 会话当前的模型 id（add-acp-model-selection）：spawn 时由 driver 上报的
    /// configOptions 填充；SetModel 成功后更新。`None` = agent 无模型选项。
    /// 内存层字段；`add-state-store` 落地后收编为其 sessions 表 `current_model`
    /// 列（review R1）——本 change 先落内存/MappingDto 层。
    pub current_model: Option<String>,
    /// 该 ACP 会话可选的模型 id 列表（来自 agent 的 configOptions）。webui
    /// 创建会话下拉的数据源；`None`/空 = 无模型选择面。
    pub available_models: Option<Vec<String>>,
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
            desired_mode: None,
            effective_mode: None,
            acp_session_id: None,
            current_model: None,
            available_models: None,
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
            desired_mode: None,
            effective_mode: None,
            acp_session_id,
            current_model: None,
            available_models: None,
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
            desired_mode: None,
            effective_mode: None,
            acp_session_id: None,
            current_model: None,
            available_models: None,
        }
    }

    /// A Spawning placeholder created eagerly with a 0-turn session request
    /// (no prompt yet): remember the requested kind/model so the first message
    /// spawns the right agent (0-turn 会话修复，P2）。`mode`（
    /// add-agent-mode-selection）同 model：占位记住、首条消息 spawn 时消费，
    /// 并记为 desired mode 供快照暴露。普通 spawn 流程直接消费时这些字段
    /// 保持 None（走默认 kind / agent 默认模型 / agent 默认行为）。
    /// `awaiting_first_prompt = true` 是占位身份本身（D1）。
    pub fn spawning_with(
        kind: Option<String>,
        model: Option<String>,
        mode: Option<String>,
    ) -> Self {
        Self {
            state: MappingState::Spawning {
                pending: Vec::new(),
                awaiting_first_prompt: true,
            },
            last_active_unix: crate::engine::now_unix(),
            project_dir: None,
            pending_kind: kind,
            pending_model: model,
            pending_mode: mode.clone(),
            desired_mode: mode,
            effective_mode: None,
            acp_session_id: None,
            current_model: None,
            available_models: None,
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
            desired_mode: None,
            effective_mode: None,
            acp_session_id: None,
            current_model: None,
            available_models: None,
        }
    }

    /// spawn 失败的映射：保留为可见的 spawn-failed 行，并带一条 transcript
    /// 错误的合成寻址 id（fail-fast-on-startup-errors 3.1）。
    pub fn spawn_failed(reason: impl Into<String>) -> Self {
        Self {
            state: MappingState::SpawnFailed {
                session_id: next_failed_id(),
                reason: reason.into(),
            },
            last_active_unix: crate::engine::now_unix(),
            project_dir: None,
            pending_kind: None,
            pending_model: None,
            pending_mode: None,
            desired_mode: None,
            effective_mode: None,
            acp_session_id: None,
            current_model: None,
            available_models: None,
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
    /// 条目挂在它下面）。其余状态没有可寻址的 transcript。
    pub fn transcript_id(&self) -> Option<&str> {
        match &self.state {
            MappingState::Active { session_id } => Some(session_id),
            MappingState::SpawnFailed { session_id, .. } => Some(session_id),
            MappingState::Spawning { .. } | MappingState::Dormant { .. } => None,
        }
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
    /// 记住、首条消息 spawn 时消费。Only the `Fresh`/`ReplacedActive` insert
    /// carries the new fields; an already-spawning placeholder keeps its
    /// existing ones.
    pub async fn begin_spawn_with(
        &self,
        key: ChannelKey,
        kind: Option<String>,
        model: Option<String>,
        mode: Option<String>,
    ) -> Result<BeginSpawn, DispatchError> {
        let mut g = self.inner.write().await;
        match g.get(&key) {
            Some(m) if matches!(m.state, MappingState::Spawning { .. }) => {
                Ok(BeginSpawn::AlreadySpawning)
            }
            Some(_) => {
                self.clear_queue(&key).await;
                g.insert(key, Mapping::spawning_with(kind, model, mode));
                Ok(BeginSpawn::ReplacedActive)
            }
            None => {
                if g.len() >= self.capacity {
                    return Err(DispatchError::Capacity(self.capacity));
                }
                g.insert(key, Mapping::spawning_with(kind, model, mode));
                Ok(BeginSpawn::Fresh)
            }
        }
    }

    /// （add-agent-mode-selection）记录操作者期望的 mode（创建请求携带或
    /// 中途切换）。与 `set_current_model` 同一模式：仅改映射，publish 由
    /// engine 层调用方完成。
    pub async fn set_desired_mode(&self, key: &ChannelKey, mode: Option<String>) {
        let mut g = self.inner.write().await;
        if let Some(m) = g.get_mut(key) {
            m.desired_mode = mode;
        }
    }

    /// （add-agent-mode-selection）记录执行体回报的实际生效 mode（本机 =
    /// spawn argv 应用值 / `ModeChanged`；远端 = 节点回报）。
    pub async fn set_effective_mode(&self, key: &ChannelKey, mode: Option<String>) {
        let mut g = self.inner.write().await;
        if let Some(m) = g.get_mut(key) {
            m.effective_mode = mode;
        }
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
        let failed = Mapping::spawn_failed(reason);
        let transcript_id = match &failed.state {
            MappingState::SpawnFailed { session_id, .. } => session_id.clone(),
            _ => unreachable!("spawn_failed constructs SpawnFailed"),
        };
        g.insert(key.clone(), failed);
        Some(transcript_id)
    }

    pub async fn insert(&self, key: ChannelKey, mapping: Mapping) -> Result<(), DispatchError> {
        let mut g = self.inner.write().await;
        if !g.contains_key(&key) && g.len() >= self.capacity {
            return Err(DispatchError::Capacity(self.capacity));
        }
        g.insert(key, mapping);
        Ok(())
    }

    /// Set the project_dir on an existing mapping. Used by WebUI to record
    /// the working directory after spawning a project session.
    pub async fn set_project_dir(&self, key: &ChannelKey, project_dir: Option<String>) {
        let mut g = self.inner.write().await;
        if let Some(m) = g.get_mut(key) {
            m.project_dir = project_dir;
        }
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
        let mut g = self.inner.write().await;
        if let Some(m) = g.get_mut(key) {
            m.current_model = Some(model_id);
        }
    }

    /// Preserve a (routing id ↔ real ACP session id) mapping as a dormant
    /// record so a conversation is not lost when a resume falls back fresh
    /// (acp-session-mapping D4: "原映射保留在存储，旧会话仍可被未来 load
    /// 寻址，不因一次失败而抹除"). The record is parked under a deterministic
    /// synthesized `closed-<hash(session_id)>` key so it survives a daemon
    /// restart in `dump_json` yet stays out of the user's chat keys (a
    /// `closed-*` chat can never collide with a web/feishu key, and the WebUI
    /// session list already renders Dormant rows). Idempotent: the same
    /// session id reuses the same archive key instead of duplicating rows.
    pub async fn preserve_closed_mapping(&self, session_id: &str, acp_session_id: Option<String>) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        session_id.hash(&mut h);
        let archive_key = ChannelKey::new("web", format!("closed-{:016x}", h.finish()));
        let mut g = self.inner.write().await;
        // 已存在则只补 last_active——不覆盖已记录的 acp_session_id。
        match g.get_mut(&archive_key) {
            Some(m) => {
                m.last_active_unix = crate::engine::now_unix();
            }
            None => {
                let mut m = Mapping::dormant(session_id.to_string(), crate::engine::now_unix());
                m.acp_session_id = acp_session_id;
                if g.len() < self.capacity {
                    g.insert(archive_key, m);
                } else {
                    tracing::warn!(%session_id, "session map at capacity; cannot archive closed mapping");
                }
            }
        }
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
        let mut g = self.inner.write().await;
        if let Some(k) = g
            .iter()
            .find(|(_, m)| m.session_id() == Some(session_id))
            .map(|(k, _)| k.clone())
        {
            g.remove(&k);
            // Session torn down: drop queued turns so they never drain into a
            // future session for the same chat key.
            self.clear_queue(&k).await;
        }
    }

    /// Remove the mapping for a specific `ChannelKey`, regardless of state.
    /// Used by the WebUI close path to drop Spawning placeholders that have
    /// no session_id (so `remove_by_session` cannot find them).
    pub async fn remove_by_key(&self, key: &ChannelKey) {
        let mut g = self.inner.write().await;
        if g.remove(key).is_some() {
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

    /// Persist Active AND Dormant entries; Spawning placeholders are never
    /// persisted (their child is tied to this process).
    /// Return a snapshot of all current mappings. Used by the WebUI to render
    /// the session list. Returns a `Vec<(ChannelKey, Mapping)>` so callers
    /// can iterate without holding the lock.
    pub async fn snapshot_all(&self) -> Vec<(ChannelKey, Mapping)> {
        let g = self.inner.read().await;
        g.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Persist Active AND Dormant entries; Spawning placeholders are never
    /// persisted (their child is tied to this process).
    ///
    /// On-disk format (v2, "structured keys"): every map key is the compact
    /// JSON string of the full `ChannelKey` object,
    /// `"{\"channel\":\"feishu\",\"reference\":\"oc_x\\u0000t1\"}"`. The core
    /// never splits the reference — the feishu adapter owns the `chat\0thread`
    /// composite encoding. [`SessionMap::restore_json`] still reads the
    /// legacy v1 flat keys (`"oc_x"`, `"oc_x\0t1"`) and promotes them to the
    /// feishu channel.
    pub async fn dump_json(&self) -> serde_json::Result<String> {
        let g = self.inner.read().await;
        let mut out = serde_json::Map::new();
        for (k, m) in g.iter() {
            // 0-turn 占位没有 routing id，但占位身份必须活过重启
            // （workbench-agent-wire-fix spec：placeholder marker survives
            // restart）——以空 session_id + awaiting_first_prompt=true 落盘。
            let is_placeholder = matches!(
                &m.state,
                MappingState::Spawning {
                    awaiting_first_prompt: true,
                    ..
                }
            );
            if let Some(sid) = m.persisted_id().or(is_placeholder.then_some("")) {
                // `serde_json::Map` keys are strings; the ChannelKey's own
                // serde produces an object, so we stringify that object as the
                // map key (self-consistent with `restore_json`'s parser).
                let key_str =
                    serde_json::to_string(k).expect("ChannelKey serializes to a JSON object");
                let dto = MappingDto {
                    session_id: sid.to_string(),
                    last_active_unix: m.last_active_unix,
                    acp_session_id: m.acp_session_id.clone(),
                    current_model: m.current_model.clone(),
                    pending_kind: m.pending_kind.clone(),
                    pending_model: m.pending_model.clone(),
                    pending_mode: m.pending_mode.clone(),
                    desired_mode: m.desired_mode.clone(),
                    project_dir: m.project_dir.clone(),
                    awaiting_first_prompt: is_placeholder,
                };
                out.insert(
                    key_str,
                    serde_json::to_value(&dto).expect("MappingDto serializes"),
                );
            }
        }
        serde_json::to_string(&out)
    }

    pub fn restore_json(s: &str) -> serde_json::Result<Self> {
        Self::restore_json_with_capacity(s, usize::MAX)
    }

    /// Restore from the on-disk shape. Every entry comes back `Dormant`:
    /// its child process died with the previous daemon, so the mapping is
    /// only good for a lazy respawn (openspec/specs/session-lifecycle/spec.md) — routing treats it as
    /// dead until the first inbound text respawns it.
    ///
    /// Keys are parsed by [`parse_disk_key`]: structured v2 keys (the compact
    /// `ChannelKey` object JSON string), plus legacy v1 flat keys (`oc_x`,
    /// `oc_x\0t1`) which become feishu-channel references.
    pub fn restore_json_with_capacity(s: &str, capacity: usize) -> serde_json::Result<Self> {
        let raw: serde_json::Map<String, serde_json::Value> = serde_json::from_str(s)?;
        let mut map = HashMap::with_capacity(raw.len());
        for (key_str, v) in raw {
            let key = parse_disk_key(&key_str).ok_or_else(|| {
                serde_json::Error::custom(format!("unparseable session state key {key_str:?}"))
            })?;
            let dto: MappingDto = serde_json::from_value(v).map_err(|e| {
                serde_json::Error::custom(format!("bad entry for {key_str:?}: {e}"))
            })?;
            let mut m = if dto.awaiting_first_prompt && dto.session_id.is_empty() {
                // 0-turn 占位（workbench-agent-wire-fix D1）：重启后仍是等待
                // 首条消息的占位，首条消息照常触发 spawn。
                Mapping::spawning_with(
                    dto.pending_kind.clone(),
                    dto.pending_model.clone(),
                    dto.pending_mode.clone(),
                )
            } else {
                Mapping::dormant(dto.session_id, dto.last_active_unix)
            };
            // Legacy records (no `acp_session_id` field) restore as
            // `None` — a later resume falls back to fresh (D4).
            m.acp_session_id = dto.acp_session_id;
            // 上次模型（内存层字段；state-store 落地后转 sessions 表列）。
            m.current_model = dto.current_model;
            // 创建时绑定的执行后端 kind（add-composer-agent-binding）；
            // 旧文件无该字段 → None → UI 显示默认 kind。
            m.pending_kind = dto.pending_kind;
            m.pending_model = dto.pending_model;
            // 创建时请求的 mode 与操作者期望的 mode（add-agent-mode-selection）；
            // 旧文件无该字段 → None → agent 默认行为。
            m.pending_mode = dto.pending_mode;
            m.desired_mode = dto.desired_mode;
            m.project_dir = dto.project_dir;
            map.insert(key, m);
        }
        Ok(Self {
            inner: Arc::new(RwLock::new(map)),
            turn_queue: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(std::sync::Mutex::new(HashMap::new())),
            delivered: Arc::new(std::sync::Mutex::new(HashMap::new())),
            capacity,
        })
    }
}

/// Parse one session-state map key into a [`ChannelKey`]:
///
/// 1. **Structured (v2)**: a JSON object string `{"channel","reference"}`
///    (the exact shape `dump_json` writes) — used verbatim.
/// 2. **Legacy (v1)**: a bare flat key — `"oc_x"` or `"oc_x\0t1"` — whose
///    `chat\0thread` composite is now the feishu channel's opaque reference
///    (`ChannelKey::feishu` keeps the composite byte-identical, so the
///    reference round-trips unchanged).
///
/// Real feishu chat ids never start with `{`, so the structured-vs-legacy
/// discrimination is unambiguous in practice.
fn parse_disk_key(key_str: &str) -> Option<ChannelKey> {
    if let Ok(k) = serde_json::from_str::<ChannelKey>(key_str) {
        return Some(k);
    }
    // Legacy flat key → feishu channel with the raw string as its reference
    // (`chat\0thread` composite preserved inside the reference, adapter-owned).
    Some(ChannelKey::feishu(key_str, None))
}

impl Default for SessionMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Legacy on-disk shape, extended with the real ACP session id
/// (acp-session-mapping D3; the `add-state-store` SQLite sessions table takes
/// this same column later — review R1) and with the 0-turn placeholder
/// marker (workbench-agent-wire-fix D1: a placeholder survives a restart so
/// its first post-restart message still spawns the child).
#[derive(Serialize, Deserialize)]
struct MappingDto {
    /// Live routing id; the empty string marks a 0-turn placeholder
    /// (`awaiting_first_prompt = true`, no child has ever existed).
    session_id: String,
    last_active_unix: i64,
    /// The agent's real ACP session id (native-ACP agents). `#[serde(default)]`
    /// keeps legacy `state.json` files (no field) readable → `None`.
    #[serde(default)]
    acp_session_id: Option<String>,
    /// 上次生效的模型 id（add-acp-model-selection；`add-state-store` 的
    /// `current_model` 列收编前先落这里）。`#[serde(default)]` 兼容旧文件。
    #[serde(default)]
    current_model: Option<String>,
    /// 创建时绑定的执行后端 kind（add-composer-agent-binding；`None` =
    /// 默认 kind）。`#[serde(default)]` 兼容旧文件。
    #[serde(default)]
    pending_kind: Option<String>,
    /// 创建时请求的模型 id（0-turn 占位记住，首条消息触发 spawn 时消费）。
    /// `#[serde(default)]` 兼容旧文件。
    #[serde(default)]
    pending_model: Option<String>,
    /// 创建时请求的 mode（0-turn 占位记住，首条消息触发 spawn 时消费；
    /// add-agent-mode-selection）。`#[serde(default)]` 兼容旧文件。
    #[serde(default)]
    pending_mode: Option<String>,
    /// 操作者期望的会话 mode（重启后快照仍可显示）。`#[serde(default)]`
    /// 兼容旧文件。
    #[serde(default)]
    desired_mode: Option<String>,
    /// 该项目记住的默认会话目录（0-turn 占位的 project_dir，spawn 时用）。
    /// `#[serde(default)]` 兼容旧文件。
    #[serde(default)]
    project_dir: Option<String>,
    /// 0-turn 占位标记（workbench-agent-wire-fix D1）。`true` + 空
    /// `session_id` = 重启后仍是等待首条消息的占位。`#[serde(default)]`
    /// 兼容旧文件（旧记录一律视为非占位）。
    #[serde(default)]
    awaiting_first_prompt: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// add-composer-agent-binding 1.2：pending_kind 落盘 → 重载保留；
    /// 旧文件（无该字段）→ None（UI 回退默认 kind 标签）。
    #[tokio::test]
    async fn pending_kind_round_trips_through_disk_shape() {
        let map = SessionMap::new();
        let mut m = Mapping::dormant("s1", 1);
        m.pending_kind = Some("claude".into());
        map.insert(ChannelKey::new("web", "web-kind"), m)
            .await
            .unwrap();
        let mut bare = Mapping::dormant("s2", 2);
        bare.pending_kind = None;
        map.insert(ChannelKey::new("web", "web-bare"), bare)
            .await
            .unwrap();

        let json = map.dump_json().await.unwrap();
        assert!(
            json.contains("\"pending_kind\":\"claude\""),
            "dump carries pending_kind, got: {json}"
        );

        let restored = SessionMap::restore_json(&json).unwrap();
        assert_eq!(
            restored
                .get(&ChannelKey::new("web", "web-kind"))
                .await
                .unwrap()
                .pending_kind
                .as_deref(),
            Some("claude"),
            "restore keeps the bound kind"
        );
        assert_eq!(
            restored
                .get(&ChannelKey::new("web", "web-bare"))
                .await
                .unwrap()
                .pending_kind,
            None,
        );
    }

    #[tokio::test]
    async fn legacy_record_without_pending_kind_restores_as_none() {
        let json = r#"{"oc_legacy":{"session_id":"s-old","last_active_unix":1}}"#;
        let map = SessionMap::restore_json(json).unwrap();
        let key = ChannelKey::feishu("oc_legacy", None);
        let m = map.get(&key).await.unwrap();
        assert_eq!(m.pending_kind, None, "legacy files read as default kind");
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
