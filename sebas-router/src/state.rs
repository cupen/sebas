use crate::error::RouterError;
use sebas_channels::ChannelKey;
use serde::{Deserialize, Serialize};
use serde::de::Error as _;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;

/// A queued turn waiting to be processed by the per-turn reply handler.
#[derive(Debug, Clone)]
pub struct QueuedTurn {
    pub prompt: String,
    pub reply_to: Option<String>,
    pub priority: bool,
}

/// In-memory session mapping state. `Spawning` is a placeholder inserted
/// synchronously when the first text arrives, so a second text racing the
/// (slow) ACP spawn is queued instead of spawning a duplicate child.
/// `Spawning` is never persisted (the child is gone after a restart anyway).
/// `Dormant` is the inverse: a mapping restored from the state file after a
/// daemon restart — the session_id is known but no child process is alive.
/// The first inbound text lazily respawns it (openspec/specs/session-lifecycle/spec.md); `Dormant` never
/// appears at runtime except via `restore_json`.
#[derive(Debug, Clone)]
pub enum MappingState {
    Spawning { pending: Vec<String> },
    Active { session_id: String },
    Dormant { session_id: String },
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
            last_active_unix: crate::router::now_unix(),
            project_dir: None,
            pending_kind: None,
            pending_model: None,
            acp_session_id: None,
            current_model: None,
            available_models: None,
        }
    }

    /// [`Mapping::active`] plus the real ACP session id (fresh spawn /
    /// successful resume mapping write — acp-session-mapping D3).
    pub fn active_with_acp(
        session_id: impl Into<String>,
        acp_session_id: Option<String>,
    ) -> Self {
        Self {
            state: MappingState::Active {
                session_id: session_id.into(),
            },
            last_active_unix: crate::router::now_unix(),
            project_dir: None,
            pending_kind: None,
            pending_model: None,
            acp_session_id,
            current_model: None,
            available_models: None,
        }
    }

    pub fn spawning() -> Self {
        Self {
            state: MappingState::Spawning {
                pending: Vec::new(),
            },
            last_active_unix: crate::router::now_unix(),
            project_dir: None,
            pending_kind: None,
            pending_model: None,
            acp_session_id: None,
            current_model: None,
            available_models: None,
        }
    }

    /// A Spawning placeholder created eagerly with a 0-turn session request
    /// (no prompt yet): remember the requested kind/model so the first message
    /// spawns the right agent (0-turn 会话修复，P2）。普通 spawn 流程直接
    /// 消费时这些字段保持 None（走默认 kind / agent 默认模型）。
    pub fn spawning_with(kind: Option<String>, model: Option<String>) -> Self {
        Self {
            state: MappingState::Spawning {
                pending: Vec::new(),
            },
            last_active_unix: crate::router::now_unix(),
            project_dir: None,
            pending_kind: kind,
            pending_model: model,
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
            acp_session_id: None,
            current_model: None,
            available_models: None,
        }
    }

    /// Live routing id — `Some` only for `Active` (a child process exists).
    /// `Dormant` deliberately returns `None` so liveness checks
    /// (`session_alive`, button-callback routing) treat it as dead.
    pub fn session_id(&self) -> Option<&str> {
        match &self.state {
            MappingState::Active { session_id } => Some(session_id),
            MappingState::Spawning { .. } | MappingState::Dormant { .. } => None,
        }
    }

    /// Id worth persisting — `Active` and `Dormant` both survive a restart
    /// (Dormant is what a persisted-then-restored Active becomes).
    fn persisted_id(&self) -> Option<&str> {
        match &self.state {
            MappingState::Active { session_id } | MappingState::Dormant { session_id } => {
                Some(session_id)
            }
            MappingState::Spawning { .. } => None,
        }
    }
}

/// What the router should do with an inbound text, decided atomically under
/// a single write lock (no check-then-act window).
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
    /// A spawn is already in flight; the prompt was queued (or dropped with a
    /// warning when the queue is full).
    Enqueued,
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

#[derive(Clone)]
pub struct SessionMap {
    inner: Arc<RwLock<HashMap<ChannelKey, Mapping>>>,
    turn_queue: Arc<RwLock<HashMap<ChannelKey, VecDeque<QueuedTurn>>>>,
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
            capacity: cap,
        }
    }

    /// Atomic check+act for an inbound text (D8 fix). Also touches
    /// `last_active_unix` on every hit so the timestamp reflects real use.
    pub async fn route_text(
        &self,
        key: ChannelKey,
        prompt: String,
    ) -> Result<TextRoute, RouterError> {
        let mut g = self.inner.write().await;
        match g.get_mut(&key) {
            None => {
                if g.len() >= self.capacity {
                    return Err(RouterError::Capacity(self.capacity));
                }
                g.insert(key, Mapping::spawning());
                Ok(TextRoute::SpawnNew)
            }
            Some(m) => {
                m.last_active_unix = crate::router::now_unix();
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
                        };
                        Ok(TextRoute::Resume(old))
                    }
                    MappingState::Spawning { pending } => {
                        if m.pending_kind.is_some() || m.pending_model.is_some() {
                            // A 0-turn placeholder (created with no prompt)
                            // awaits its first message: flip it to a plain
                            // spawning placeholder (the pending kind/model are
                            // consumed by the SpawnNew caller) and hand the
                            // message to the spawn path (P2 fix). Without
                            // this, the first message would be queued and no
                            // child would ever spawn.
                            m.state = MappingState::Spawning {
                                pending: Vec::new(),
                            };
                            Ok(TextRoute::SpawnNew)
                        } else if pending.len() < MAX_PENDING {
                            pending.push(prompt);
                            Ok(TextRoute::Enqueued)
                        } else {
                            tracing::warn!("pending queue full; dropping newest message");
                            Ok(TextRoute::Enqueued)
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
    pub async fn begin_spawn(&self, key: ChannelKey) -> Result<BeginSpawn, RouterError> {
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
                    return Err(RouterError::Capacity(self.capacity));
                }
                g.insert(key, Mapping::spawning());
                Ok(BeginSpawn::Fresh)
            }
        }
    }

    /// [`SessionMap::begin_spawn`] plus a requested kind/model to remember on
    /// the placeholder (0-turn sessions: the first message spawns the right
    /// agent — P2 fix). Only the `Fresh`/`ReplacedActive` insert carries the
    /// new fields; an already-spawning placeholder keeps its existing ones.
    pub async fn begin_spawn_with(
        &self,
        key: ChannelKey,
        kind: Option<String>,
        model: Option<String>,
    ) -> Result<BeginSpawn, RouterError> {
        let mut g = self.inner.write().await;
        match g.get(&key) {
            Some(m) if matches!(m.state, MappingState::Spawning { .. }) => {
                Ok(BeginSpawn::AlreadySpawning)
            }
            Some(_) => {
                self.clear_queue(&key).await;
                g.insert(key, Mapping::spawning_with(kind, model));
                Ok(BeginSpawn::ReplacedActive)
            }
            None => {
                if g.len() >= self.capacity {
                    return Err(RouterError::Capacity(self.capacity));
                }
                g.insert(key, Mapping::spawning_with(kind, model));
                Ok(BeginSpawn::Fresh)
            }
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
                m.last_active_unix = crate::router::now_unix();
                let mut next = MappingState::Active { session_id };
                // 写入映射：新会话/成功 resume 的 acp_session_id 落盘给后续
                // resume 使用；把旧字段就地一起换掉（D4：load 失败时旧映射
                // 保持不动——本次 activate 携带的是新会话的 id）。
                std::mem::swap(&mut m.state, &mut next);
                let pending = match next {
                    MappingState::Spawning { pending } => pending,
                    MappingState::Active { .. } | MappingState::Dormant { .. } => Vec::new(),
                };
                m.acp_session_id = acp_session_id;
                m.current_model = model.as_ref().map(|info| info.current.clone());
                m.available_models = model.map(|info| info.options);
                pending
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

    /// Spawn failed: remove the placeholder (queued prompts drop with it).
    /// Only touches Spawning entries — never an Active session.
    pub async fn fail_spawn(&self, key: &ChannelKey) {
        let mut g = self.inner.write().await;
        let is_spawning = matches!(
            g.get(key).map(|m| &m.state),
            Some(MappingState::Spawning { .. })
        );
        if is_spawning {
            g.remove(key);
        }
    }

    pub async fn insert(&self, key: ChannelKey, mapping: Mapping) -> Result<(), RouterError> {
        let mut g = self.inner.write().await;
        if !g.contains_key(&key) && g.len() >= self.capacity {
            return Err(RouterError::Capacity(self.capacity));
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
        let archive_key = ChannelKey::new("web", &format!("closed-{:016x}", h.finish()));
        let mut g = self.inner.write().await;
        // 已存在则只补 last_active——不覆盖已记录的 acp_session_id。
        match g.get_mut(&archive_key) {
            Some(m) => {
                m.last_active_unix = crate::router::now_unix();
            }
            None => {
                let mut m = Mapping::dormant(session_id.to_string(), crate::router::now_unix());
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
    pub async fn clear_queue(&self, key: &ChannelKey) {
        self.turn_queue.write().await.remove(key);
    }

    /// Enqueue a turn for the given session. Priority turns are inserted at
    /// the front; non-priority turns are appended to the back.
    pub async fn enqueue_turn(&self, key: &ChannelKey, turn: QueuedTurn) {
        let mut q = self.turn_queue.write().await;
        let deque = q.entry(key.clone()).or_insert_with(VecDeque::new);
        if turn.priority {
            deque.push_front(turn);
        } else {
            deque.push_back(turn);
        }
    }

    /// Pop the next turn from the queue, if any.
    pub async fn pop_next_turn(&self, key: &ChannelKey) -> Option<QueuedTurn> {
        let mut q = self.turn_queue.write().await;
        let popped = q.get_mut(key).and_then(|deque| deque.pop_front());
        if let Some(deque) = q.get(key)
            && deque.is_empty()
        {
            q.remove(key);
        }
        popped
    }

    /// Return the number of queued turns for the given session.
    pub async fn queue_len(&self, key: &ChannelKey) -> usize {
        let q = self.turn_queue.read().await;
        q.get(key).map(|deque| deque.len()).unwrap_or(0)
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
            if let Some(sid) = m.persisted_id() {
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
            let dto: MappingDto =
                serde_json::from_value(v).map_err(|e| {
                    serde_json::Error::custom(format!("bad entry for {key_str:?}: {e}"))
                })?;
            let mut m = Mapping::dormant(dto.session_id, dto.last_active_unix);
            // Legacy records (no `acp_session_id` field) restore as
            // `None` — a later resume falls back to fresh (D4).
            m.acp_session_id = dto.acp_session_id;
            // 上次模型（内存层字段；state-store 落地后转 sessions 表列）。
            m.current_model = dto.current_model;
            map.insert(key, m);
        }
        Ok(Self {
            inner: Arc::new(RwLock::new(map)),
            turn_queue: Arc::new(RwLock::new(HashMap::new())),
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
/// this same column later — review R1).
#[derive(Serialize, Deserialize)]
struct MappingDto {
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
}
