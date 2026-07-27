use crate::error::RouterError;
use feishu::events::SessionKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mapping {
    pub session_id: String,
    pub last_active_unix: i64,
}

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

    /// Reverse index: find the `SessionKey` that maps to a given `session_id`.
    /// Used to route ACP events (which only carry a `session_id`) back to the
    /// Feishu chat that owns the session — e.g. permission-request cards.
    pub async fn lookup_key_by_session(&self, session_id: &str) -> Option<SessionKey> {
        self.inner
            .read()
            .await
            .iter()
            .find(|(_, m)| m.session_id == session_id)
            .map(|(k, _)| k.clone())
    }

    /// Remove a mapping by `session_id` (used when a session is killed).
    pub async fn remove_by_session(&self, session_id: &str) {
        let mut g = self.inner.write().await;
        if let Some(k) = g
            .iter()
            .find(|(_, m)| m.session_id == session_id)
            .map(|(k, _)| k.clone())
        {
            g.remove(&k);
        }
    }

    pub async fn dump_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(&*self.inner.read().await)
    }

    pub fn restore_json(s: &str) -> serde_json::Result<Self> {
        let map: HashMap<SessionKey, Mapping> = serde_json::from_str(s)?;
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
