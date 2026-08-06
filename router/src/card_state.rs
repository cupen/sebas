//! 卡片流累积状态（spec §4.1）。纯状态，并行于 `MsgIdMap`：
//! `session_id -> CardState`。渲染与 FSM 在 router.rs / cards.rs。

use feishu::cards::CardElement;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Phase token: string IS the Feishu `emoji_type` (e.g. `"Typing"`, `"OnIt"`,
/// `"DONE"`, `"CrossMark"`) — required because Feishu's reaction API rejects
/// arbitrary Unicode emoji like 👀/🚧/✅/❌ with error 231001. The display
/// glyph for the card header is derived via `feishu::cards::phase_visual`.
pub mod phase {
    pub const SEED: &str = "Typing"; // watching / waiting on first event
    pub const WORKING: &str = "OnIt"; // streaming response in progress
    pub const DONE: &str = "DONE"; // Finished event
    pub const FAILED: &str = "CrossMark"; // terminal Error event
}

#[derive(Debug, Clone)]
pub struct CardState {
    pub user_prompt: String,
    pub status_emoji: String,
    pub body: Vec<CardElement>,
}

impl CardState {
    /// seed_card 用：记录真实 user_prompt（重渲染引用块用），emoji SEED，空 body。
    pub fn new(user_prompt: &str) -> Self {
        Self {
            user_prompt: user_prompt.into(),
            status_emoji: phase::SEED.into(),
            body: Vec::new(),
        }
    }

    /// 早到事件兜底：prompt=""，emoji SEED，空 body（spec §4.2 lazy seed）。
    pub fn lazy() -> Self {
        Self {
            user_prompt: String::new(),
            status_emoji: phase::SEED.into(),
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
        g.entry(session_id)
            .or_insert_with(|| CardState::new(&user_prompt));
    }

    /// 无 entry 时 `lazy()` 兜底插入，再对 `&mut CardState` 跑 `f`，返回其结果。
    pub async fn apply<F, R>(&self, session_id: &str, f: F) -> R
    where
        F: FnOnce(&mut CardState) -> R,
    {
        let mut g = self.inner.write().await;
        let st = g
            .entry(session_id.to_string())
            .or_insert_with(CardState::lazy);
        f(st)
    }

    /// 克隆一份给 flush 渲染。
    pub async fn snapshot(&self, session_id: &str) -> Option<CardState> {
        self.inner.read().await.get(session_id).cloned()
    }

    /// session 死亡/通道关时移除（防无界增长）。
    pub async fn drop(&self, session_id: &str) {
        self.inner.write().await.remove(session_id);
    }

    /// Returns the current status emoji for the given session, or None if
    /// the session has no CardState entry.
    pub async fn status_emoji(&self, session_id: &str) -> Option<String> {
        self.inner
            .read()
            .await
            .get(session_id)
            .map(|st| st.status_emoji.clone())
    }
}
