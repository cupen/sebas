//! 卡片流累积状态（spec §4.1）。纯状态，并行于 `MsgIdMap`：
//! `session_id -> CardState`。渲染与 FSM 在 router.rs / cards.rs。

use feishu::cards::CardElement;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Phase token: string IS the Feishu `emoji_type` (e.g. `"Typing"`, `"OnIt"`,
/// `"DONE"`, `"CrossMark"`) — required because Feishu's reaction API rejects
//// arbitrary Unicode emoji like 👀/🚧/✅/❌ with error 231001. These values
/// surface as reactions on the root card to reflect session state; the card
/// header title is the topic derived from the prompt instead
/// (`feishu::cards::derive_topic`).
pub mod phase {
    pub const SEED: &str = "Typing"; // watching / waiting on first event
    pub const WORKING: &str = "OnIt"; // streaming response in progress
    pub const DONE: &str = "DONE"; // Finished event
    pub const FAILED: &str = "CrossMark"; // terminal Error event
}

/// Accumulated token usage for a session. Reset at the start of each turn
/// (round) so the footer can show per-round and cumulative totals.
/// Re-exported from feishu::cards for convenience.
pub use feishu::cards::CardFooter;

#[derive(Debug, Clone)]
pub struct CardState {
    pub user_prompt: String,
    pub status_emoji: String,
    /// Session start timestamp — used to compute elapsed time for the
    /// parent panel title ("🤔 折腾中 · 3项 · 45s").
    pub started_at: Instant,
    pub body: Vec<CardElement>,
    /// Model name and token usage tracking.
    pub usage: CardFooter,
}

impl CardState {
    /// seed_card 用：记录真实 user_prompt（重渲染引用块用），emoji SEED，空 body。
    pub fn new(user_prompt: &str) -> Self {
        Self {
            user_prompt: user_prompt.into(),
            status_emoji: phase::SEED.into(),
            started_at: Instant::now(),
            body: Vec::new(),
            usage: CardFooter::default(),
        }
    }

    /// 早到事件兜底：prompt=""，emoji SEED，空 body（spec §4.2 lazy seed）。
    pub fn lazy() -> Self {
        Self {
            user_prompt: String::new(),
            status_emoji: phase::SEED.into(),
            started_at: Instant::now(),
            body: Vec::new(),
            usage: CardFooter::default(),
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

    /// Return a snapshot of all card states. Used by the WebUI.
    pub async fn snapshot_all(&self) -> HashMap<String, CardState> {
        self.inner.read().await.clone()
    }

    /// 替换 session 的 body，重置 started_at。用于卡片换卡（rotate_card）后清空 body。
    pub async fn reset_body(&self, session_id: &str, elements: Vec<CardElement>) {
        let mut g = self.inner.write().await;
        if let Some(st) = g.get_mut(session_id) {
            st.body = elements;
            st.started_at = Instant::now();
        }
    }
}