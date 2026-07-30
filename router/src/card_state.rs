//! 卡片流累积状态（spec §4.1）。纯状态，并行于 `MsgIdMap`：
//! `session_id -> CardState`。渲染与 FSM 在 router.rs / cards.rs。

use feishu::cards::CardElement;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct CardState {
    pub user_prompt: String,
    pub status_emoji: String,
    pub body: Vec<CardElement>,
}

impl CardState {
    /// seed_card 用：记录真实 user_prompt（重渲染引用块用），emoji 👀，空 body。
    pub fn new(user_prompt: &str) -> Self {
        Self {
            user_prompt: user_prompt.into(),
            status_emoji: "👀".into(),
            body: Vec::new(),
        }
    }

    /// 早到事件兜底：prompt=""，emoji 👀，空 body（spec §4.2 lazy seed）。
    pub fn lazy() -> Self {
        Self {
            user_prompt: String::new(),
            status_emoji: "👀".into(),
            body: Vec::new(),
        }
    }
}

#[derive(Default, Clone)]
pub struct CardStateMap {
    inner: Arc<RwLock<HashMap<String, CardState>>>,
}

impl CardStateMap {
    /// 幂等 seed：entry 已存在则保留（防 SpawnAcp 重入冲掉已累积状态）。
    pub async fn seed(&self, session_id: String, user_prompt: String) {
        let mut g = self.inner.write().await;
        g.entry(session_id).or_insert_with(|| CardState::new(&user_prompt));
    }

    /// 无 entry 时 `lazy()` 兜底插入，再对 `&mut CardState` 跑 `f`。
    pub async fn apply<F: FnOnce(&mut CardState)>(&self, session_id: &str, f: F) {
        let mut g = self.inner.write().await;
        let st = g.entry(session_id.to_string()).or_insert_with(CardState::lazy);
        f(st);
    }

    /// 克隆一份给 flush 渲染。
    pub async fn snapshot(&self, session_id: &str) -> Option<CardState> {
        self.inner.read().await.get(session_id).cloned()
    }

    /// session 死亡/通道关时移除（防无界增长）。
    pub async fn drop(&self, session_id: &str) {
        self.inner.write().await.remove(session_id);
    }
}
