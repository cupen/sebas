use crate::error::RouterError;
use feishu::events::SessionKey;
use serde::{Deserialize, Serialize};
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
/// The first inbound text lazily respawns it (spec §3.3e); `Dormant` never
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
}

impl Mapping {
    pub fn active(session_id: impl Into<String>) -> Self {
        Self {
            state: MappingState::Active {
                session_id: session_id.into(),
            },
            last_active_unix: crate::router::now_unix(),
        }
    }

    pub fn spawning() -> Self {
        Self {
            state: MappingState::Spawning {
                pending: Vec::new(),
            },
            last_active_unix: crate::router::now_unix(),
        }
    }

    pub fn dormant(session_id: impl Into<String>, last_active_unix: i64) -> Self {
        Self {
            state: MappingState::Dormant {
                session_id: session_id.into(),
            },
            last_active_unix,
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
    /// (spec §3.3e).
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
    inner: Arc<RwLock<HashMap<SessionKey, Mapping>>>,
    turn_queue: Arc<RwLock<HashMap<SessionKey, VecDeque<QueuedTurn>>>>,
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
        key: SessionKey,
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
                        // caller (spec §3.3e).
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
    pub async fn begin_spawn(&self, key: SessionKey) -> Result<BeginSpawn, RouterError> {
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
    pub async fn activate(&self, key: &SessionKey, session_id: String) -> Vec<String> {
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
    pub async fn fail_spawn(&self, key: &SessionKey) {
        let mut g = self.inner.write().await;
        let is_spawning = matches!(
            g.get(key).map(|m| &m.state),
            Some(MappingState::Spawning { .. })
        );
        if is_spawning {
            g.remove(key);
        }
    }

    pub async fn insert(&self, key: SessionKey, mapping: Mapping) -> Result<(), RouterError> {
        let mut g = self.inner.write().await;
        if !g.contains_key(&key) && g.len() >= self.capacity {
            return Err(RouterError::Capacity(self.capacity));
        }
        g.insert(key, mapping);
        Ok(())
    }

    pub async fn get(&self, key: &SessionKey) -> Option<Mapping> {
        self.inner.read().await.get(key).cloned()
    }

    pub async fn lookup_key_by_session(&self, session_id: &str) -> Option<SessionKey> {
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

    /// Drop any queued turns for a session key. Called when a session is torn
    /// down or replaced so stale prompts never drain into a future session.
    pub async fn clear_queue(&self, key: &SessionKey) {
        self.turn_queue.write().await.remove(key);
    }

    /// Enqueue a turn for the given session. Priority turns are inserted at
    /// the front; non-priority turns are appended to the back.
    pub async fn enqueue_turn(&self, key: &SessionKey, turn: QueuedTurn) {
        let mut q = self.turn_queue.write().await;
        let deque = q.entry(key.clone()).or_insert_with(VecDeque::new);
        if turn.priority {
            deque.push_front(turn);
        } else {
            deque.push_back(turn);
        }
    }

    /// Pop the next turn from the queue, if any.
    pub async fn pop_next_turn(&self, key: &SessionKey) -> Option<QueuedTurn> {
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
    pub async fn queue_len(&self, key: &SessionKey) -> usize {
        let q = self.turn_queue.read().await;
        q.get(key).map(|deque| deque.len()).unwrap_or(0)
    }

    /// Persist Active AND Dormant entries, in the legacy flat shape
    /// (`{"session_id": ..., "last_active_unix": ...}`) so the on-disk format
    /// is unchanged and restores work across versions. Spawning placeholders
    /// are never persisted (their child is tied to this process).
    pub async fn dump_json(&self) -> serde_json::Result<String> {
        let g = self.inner.read().await;
        let active: HashMap<&SessionKey, MappingDto> = g
            .iter()
            .filter_map(|(k, m)| {
                m.persisted_id().map(|sid| {
                    (
                        k,
                        MappingDto {
                            session_id: sid.to_string(),
                            last_active_unix: m.last_active_unix,
                        },
                    )
                })
            })
            .collect();
        serde_json::to_string(&active)
    }

    pub fn restore_json(s: &str) -> serde_json::Result<Self> {
        Self::restore_json_with_capacity(s, usize::MAX)
    }

    /// Restore from the on-disk shape. Every entry comes back `Dormant`:
    /// its child process died with the previous daemon, so the mapping is
    /// only good for a lazy respawn (spec §3.3e) — routing treats it as
    /// dead until the first inbound text respawns it.
    pub fn restore_json_with_capacity(s: &str, capacity: usize) -> serde_json::Result<Self> {
        let dto: HashMap<SessionKey, MappingDto> = serde_json::from_str(s)?;
        let map = dto
            .into_iter()
            .map(|(k, d)| (k, Mapping::dormant(d.session_id, d.last_active_unix)))
            .collect();
        Ok(Self {
            inner: Arc::new(RwLock::new(map)),
            turn_queue: Arc::new(RwLock::new(HashMap::new())),
            capacity,
        })
    }
}

impl Default for SessionMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Legacy on-disk shape (unchanged).
#[derive(Serialize, Deserialize)]
struct MappingDto {
    session_id: String,
    last_active_unix: i64,
}
