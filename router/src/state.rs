use crate::error::RouterError;
use feishu::events::SessionKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory session mapping state. `Spawning` is a placeholder inserted
/// synchronously when the first text arrives, so a second text racing the
/// (slow) ACP spawn is queued instead of spawning a duplicate child.
/// `Spawning` is never persisted (the child is gone after a restart anyway).
#[derive(Debug, Clone)]
pub enum MappingState {
    Spawning { pending: Vec<String> },
    Active { session_id: String },
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
            state: MappingState::Spawning { pending: Vec::new() },
            last_active_unix: crate::router::now_unix(),
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match &self.state {
            MappingState::Active { session_id } => Some(session_id),
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
    capacity: usize,
}

impl SessionMap {
    pub fn new() -> Self {
        Self::with_capacity(usize::MAX)
    }
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            capacity: cap,
        }
    }

    /// Atomic check+act for an inbound text (D8 fix).
    pub async fn route_text(&self, key: SessionKey, prompt: String) -> Result<TextRoute, RouterError> {
        let mut g = self.inner.write().await;
        match g.get_mut(&key) {
            None => {
                if g.len() >= self.capacity {
                    return Err(RouterError::Capacity(self.capacity));
                }
                g.insert(key, Mapping::spawning());
                Ok(TextRoute::SpawnNew)
            }
            Some(m) => match &mut m.state {
                MappingState::Active { session_id } => {
                    Ok(TextRoute::Continue(session_id.clone()))
                }
                MappingState::Spawning { pending } => {
                    if pending.len() < MAX_PENDING {
                        pending.push(prompt);
                    } else {
                        tracing::warn!("pending queue full; dropping newest message");
                    }
                    Ok(TextRoute::Enqueued)
                }
            },
        }
    }

    /// `/new`: unconditionally (re)place a Spawning placeholder — unless a
    /// spawn is already in flight, in which case keep the existing
    /// placeholder (and its pending queue) and report it.
    pub async fn begin_spawn(&self, key: SessionKey) -> Result<BeginSpawn, RouterError> {
        let mut g = self.inner.write().await;
        match g.get(&key) {
            Some(m) if matches!(m.state, MappingState::Spawning { .. }) => {
                Ok(BeginSpawn::AlreadySpawning)
            }
            Some(_) => {
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
    pub async fn activate(&self, key: &SessionKey, session_id: String) -> Vec<String> {
        let mut g = self.inner.write().await;
        match g.get_mut(key) {
            Some(m) => {
                match std::mem::replace(
                    &mut m.state,
                    MappingState::Active { session_id },
                ) {
                    MappingState::Spawning { pending } => pending,
                    MappingState::Active { .. } => Vec::new(),
                }
            }
            None => {
                tracing::warn!(?key, "activate without placeholder; inserting fresh mapping");
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
        }
    }

    /// Persist ONLY Active entries, in the legacy flat shape
    /// (`{"session_id": ..., "last_active_unix": ...}`) so the on-disk format
    /// is unchanged and restores work across versions.
    pub async fn dump_json(&self) -> serde_json::Result<String> {
        let g = self.inner.read().await;
        let active: HashMap<&SessionKey, MappingDto> = g
            .iter()
            .filter_map(|(k, m)| {
                m.session_id().map(|sid| {
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
        let dto: HashMap<SessionKey, MappingDto> = serde_json::from_str(s)?;
        let map = dto
            .into_iter()
            .map(|(k, d)| {
                (
                    k,
                    Mapping {
                        state: MappingState::Active {
                            session_id: d.session_id,
                        },
                        last_active_unix: d.last_active_unix,
                    },
                )
            })
            .collect();
        Ok(Self {
            inner: Arc::new(RwLock::new(map)),
            capacity: usize::MAX,
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
