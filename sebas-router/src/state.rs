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
}

impl Mapping {
    pub fn active(session_id: impl Into<String>) -> Self {
        Self {
            state: MappingState::Active {
                session_id: session_id.into(),
            },
            last_active_unix: crate::router::now_unix(),
            project_dir: None,
        }
    }

    pub fn spawning() -> Self {
        Self {
            state: MappingState::Spawning {
                pending: Vec::new(),
            },
            last_active_unix: crate::router::now_unix(),
            project_dir: None,
        }
    }

    pub fn dormant(session_id: impl Into<String>, last_active_unix: i64) -> Self {
        Self {
            state: MappingState::Dormant {
                session_id: session_id.into(),
            },
            last_active_unix,
            project_dir: None,
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
                        if pending.len() < MAX_PENDING {
                            pending.push(prompt);
                        } else {
                            tracing::warn!("pending queue full; dropping newest message");
                        }
                        Ok(TextRoute::Enqueued)
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

    /// Flip Spawning -> Active and drain queued prompts (returned in order).
    /// Also touches `last_active_unix`: activation is the moment the session
    /// starts doing real work.
    pub async fn activate(&self, key: &ChannelKey, session_id: String) -> Vec<String> {
        let mut g = self.inner.write().await;
        match g.get_mut(key) {
            Some(m) => {
                m.last_active_unix = crate::router::now_unix();
                match std::mem::replace(&mut m.state, MappingState::Active { session_id }) {
                    MappingState::Spawning { pending } => pending,
                    MappingState::Active { .. } | MappingState::Dormant { .. } => Vec::new(),
                }
            }
            None => {
                tracing::warn!(
                    ?key,
                    "activate without placeholder; inserting fresh mapping"
                );
                g.insert(key.clone(), Mapping::active(session_id));
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
            map.insert(key, Mapping::dormant(dto.session_id, dto.last_active_unix));
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

/// On-disk shape (unchanged).
#[derive(Serialize, Deserialize)]
struct MappingDto {
    session_id: String,
    last_active_unix: i64,
}
