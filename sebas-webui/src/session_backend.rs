//! The session backend seam (openspec/changes/add-core-session-channel — task 2.1).
//!
//! Mirrors the `AdminAdapter` seam: the webui crate owns the trait, the sebas
//! binary crate supplies implementations (in-process over `RouterHandle`, or
//! the core session channel socket client). The webui crate never depends on
//! the binary crate — that is the seam's whole point.
//!
//! Everything the session routes need flows through this trait: reads
//! (snapshot/turns/focus), mutations (spawn/message/close), the event
//! subscription for SSE, and the reachability report that drives honest
//! degradation rendering when the core is not connected.

use async_trait::async_trait;
use sebas_feishu::events::SessionKey;
use sebas_router::{SessionEvent, SessionInfo, TurnEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::broadcast;

/// Whether the backend can currently reach the session authority (the core),
/// and if not, why — rendered verbatim so degradation is honest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reachability {
    /// The core is reachable; session controls are live.
    Reachable,
    /// The core cannot be reached; the board renders the cause and the
    /// composer stays disabled.
    Unreachable { cause: String },
}

/// Typed rejection for a session mutation (spec: rejections name the reason;
/// nothing is mutated on rejection).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum SessionRejection {
    /// No session exists for the given key.
    UnknownSession { key: String },
    /// The requested project directory is not a usable directory.
    /// Deliberately carries no path details — no existence disclosure.
    UnusableProjectDir,
    /// The core is at its session capacity.
    Capacity { limit: usize },
    /// The request could not be delivered to the session authority.
    Unavailable { cause: String },
}

impl std::fmt::Display for SessionRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionRejection::UnknownSession { key } => write!(f, "会话不存在: {key}"),
            SessionRejection::UnusableProjectDir => {
                write!(f, "项目目录不可用（不是目录或无法访问）")
            }
            SessionRejection::Capacity { limit } => write!(f, "会话数已达上限 {limit}"),
            SessionRejection::Unavailable { cause } => write!(f, "核心不可达: {cause}"),
        }
    }
}

/// One gated tool call awaiting an operator decision (webui review card).
/// `session_id` is the encoded session key; `request_id` equals the kernel's
/// `tool_use_id` and is what [`SessionBackend::answer_permission`] takes back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionNotice {
    pub request_id: String,
    /// Encoded session key (URL-safe, as used in routes).
    pub session_id: String,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub reason: String,
}

/// The operator's answer to a [`PermissionNotice`]. `escalate` = one-shot
/// elevated retry carrying the operator's stated reason (the session policy
/// itself never widens).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PermissionDecision {
    AllowOnce,
    AllowSession,
    Deny,
    Escalate { reason: String },
}

/// The seam every session-data source must satisfy.
#[async_trait]
pub trait SessionBackend: Send + Sync {
    /// Every known session, in the shape the session rows need.
    async fn snapshot(&self) -> Vec<SessionInfo>;

    /// The currently focused session, if any.
    async fn focused(&self) -> Option<SessionKey>;

    /// Mark the focused session (idempotent; clearing with `None`).
    async fn set_focus(&self, key: Option<SessionKey>);

    /// Subscribe to session events (created / updated / removed / resync).
    /// Bounded: a lagging consumer sees `broadcast::error::RecvError::Lagged`.
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent>;

    /// Create a session, optionally rooted in a project directory. Returns
    /// the new session key. The placeholder is immediately visible in
    /// `snapshot` (Spawning) and via the event stream (Created).
    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<SessionKey, SessionRejection>;

    /// Send a message to an existing session. Unknown keys are rejected.
    async fn message(&self, key: SessionKey, message: String) -> Result<(), SessionRejection>;

    /// Close a session (kills the live child when there is one).
    async fn close(&self, key: SessionKey) -> Result<(), SessionRejection>;

    /// The session's rendered transcript at or after `from` (monotonic
    /// positions — a second call at the returned last position yields only
    /// newer entries).
    async fn turns(&self, key: SessionKey, from: u64) -> Result<Vec<TurnEntry>, SessionRejection>;

    /// Whether the session authority is reachable right now, and if not, why.
    async fn reachability(&self) -> Reachability;

    /// Live stream of gated tool calls awaiting a decision (the review-card
    /// feed). `None` = this backend has no permission interaction (its
    /// sessions never gate, or gating is surfaced elsewhere).
    fn permission_requests(&self) -> Option<broadcast::Receiver<PermissionNotice>> {
        None
    }

    /// Deliver an operator decision for `request_id`. Returns `false` when
    /// no pending request carries that id (already answered, timed out, or
    /// unknown — callers may retry briefly).
    async fn answer_permission(&self, _request_id: &str, _decision: PermissionDecision) -> bool {
        false
    }

    /// Create a session, optionally pinning the execution backend. The
    /// default ignores the hint (single-backend seams); composite seams
    /// route on it.
    async fn spawn_with(
        &self,
        prompt: String,
        project_dir: Option<String>,
        _backend: Option<&str>,
    ) -> Result<SessionKey, SessionRejection> {
        self.spawn(prompt, project_dir).await
    }
}

// ─── In-process implementation (task 2.2) ──────────────────────────────────

/// In-process backend over the router. Used by `sebas run --webui`, where the
/// webui lives in the same process as the session authority.
pub struct InProcessBackend {
    router: sebas_router::RouterHandle,
}

impl InProcessBackend {
    pub fn new(router: sebas_router::RouterHandle) -> Self {
        Self { router }
    }
}

#[async_trait]
impl SessionBackend for InProcessBackend {
    async fn snapshot(&self) -> Vec<SessionInfo> {
        self.router.session_info_snapshot().await
    }

    async fn focused(&self) -> Option<SessionKey> {
        self.router.active_session_snapshot().await
    }

    async fn set_focus(&self, key: Option<SessionKey>) {
        self.router.web_set_active(key).await;
    }

    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.router.subscribe_session_events()
    }

    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<SessionKey, SessionRejection> {
        // web_spawn never fails structurally: the placeholder is inserted and
        // the spawn failure surfaces as a Removed event later.
        Ok(self.router.web_spawn(prompt, project_dir).await)
    }

    async fn message(&self, key: SessionKey, message: String) -> Result<(), SessionRejection> {
        // Route semantics preserved: an unknown key spawns a new session (the
        // feishu inbound path behaves the same). Typed rejections apply to the
        // channel server, which pre-checks existence.
        self.router.web_send_message(key, message).await;
        Ok(())
    }

    async fn close(&self, key: SessionKey) -> Result<(), SessionRejection> {
        match self.router.web_close_session(key).await {
            sebas_router::router::CloseOutcome::Closed => Ok(()),
            sebas_router::router::CloseOutcome::NotFound => {
                Err(SessionRejection::UnknownSession {
                    key: String::new(),
                })
            }
        }
    }

    async fn turns(&self, key: SessionKey, from: u64) -> Result<Vec<TurnEntry>, SessionRejection> {
        self.router
            .session_turns(&key, from)
            .await
            .ok_or(SessionRejection::UnknownSession {
                key: key.chat_id.clone(),
            })
    }

    async fn reachability(&self) -> Reachability {
        // Same process as the authority: always reachable.
        Reachability::Reachable
    }
}

// ─── Fake backend for tests (task 2.3) ─────────────────────────────────────

/// Fake backend for tests: settable session set, in-memory transcript,
/// and an "unreachable" mode. No child process, no socket.
pub struct FakeBackend {
    inner: tokio::sync::RwLock<FakeState>,
    events: broadcast::Sender<SessionEvent>,
    reachable: std::sync::atomic::AtomicBool,
    unreachable_cause: std::sync::Mutex<Option<String>>,
    /// The next spawn index — used to mint distinct fake keys.
    next_spawn: std::sync::atomic::AtomicU64,
}

#[derive(Default)]
struct FakeState {
    sessions: Vec<SessionInfo>,
    focused: Option<SessionKey>,
    transcripts: HashMap<String, Vec<TurnEntry>>,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackend {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            inner: tokio::sync::RwLock::new(FakeState::default()),
            events,
            reachable: std::sync::atomic::AtomicBool::new(true),
            unreachable_cause: std::sync::Mutex::new(None),
            next_spawn: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Seed/replace the visible session set.
    pub async fn set_sessions(&self, sessions: Vec<SessionInfo>) {
        self.inner.write().await.sessions = sessions;
    }

    /// Append one transcript entry for `session_id` (position auto-assigned).
    pub async fn push_turn(&self, session_id: &str, kind: &str, content: &str) {
        self.push_turn_typed(session_id, kind, "markdown", content)
            .await;
    }

    /// `push_turn` with an explicit `element_type` ("markdown" | "thinking").
    pub async fn push_turn_typed(
        &self,
        session_id: &str,
        kind: &str,
        element_type: &str,
        content: &str,
    ) {
        let mut g = self.inner.write().await;
        let log = g.transcripts.entry(session_id.to_string()).or_default();
        let position = log.len() as u64;
        log.push(TurnEntry {
            position,
            kind: kind.to_string(),
            element_type: element_type.to_string(),
            content: content.to_string(),
            created_at_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        });
    }

    /// Flip reachability; `cause` is reported while unreachable.
    pub fn set_reachable(&self, reachable: bool, cause: &str) {
        self.reachable
            .store(reachable, std::sync::atomic::Ordering::SeqCst);
        *self.unreachable_cause.lock().unwrap() = Some(cause.to_string());
    }

    /// Emit an event as if the authority had published it.
    pub fn emit(&self, ev: SessionEvent) {
        let _ = self.events.send(ev);
    }

    fn key_str(key: &SessionKey) -> String {
        serde_json::to_string(key).unwrap_or_default()
    }
}

#[async_trait]
impl SessionBackend for FakeBackend {
    async fn snapshot(&self) -> Vec<SessionInfo> {
        self.inner.read().await.sessions.clone()
    }

    async fn focused(&self) -> Option<SessionKey> {
        self.inner.read().await.focused.clone()
    }

    async fn set_focus(&self, key: Option<SessionKey>) {
        self.inner.write().await.focused = key;
    }

    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<SessionKey, SessionRejection> {
        if !self.reachable.load(std::sync::atomic::Ordering::SeqCst) {
            let cause = self
                .unreachable_cause
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_default();
            return Err(SessionRejection::Unavailable { cause });
        }
        let n = self
            .next_spawn
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let key = SessionKey {
            chat_id: format!("web-fake-{n}"),
            thread_id: None,
        };
        let session = SessionInfo {
            chat_id: key.chat_id.clone(),
            thread_id: None,
            session_id: None,
            status: "spawning".into(),
            phase: None,
            user_prompt: None,
            last_active_unix: 0,
            project_dir,
        };
        let ev = SessionEvent::Created { session };
        if let SessionEvent::Created { session } = &ev {
            self.inner.write().await.sessions.push(session.clone());
        }
        self.emit(ev);
        let _ = prompt; // the fake does not model prompt-driven topic derivation
        Ok(key)
    }

    async fn message(&self, key: SessionKey, _message: String) -> Result<(), SessionRejection> {
        if !self.reachable.load(std::sync::atomic::Ordering::SeqCst) {
            let cause = self
                .unreachable_cause
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_default();
            return Err(SessionRejection::Unavailable { cause });
        }
        let exists = {
            let g = self.inner.read().await;
            g.sessions
                .iter()
                .any(|s| s.chat_id == key.chat_id && s.thread_id == key.thread_id)
        };
        if exists {
            Ok(())
        } else {
            Err(SessionRejection::UnknownSession {
                key: Self::key_str(&key),
            })
        }
    }

    async fn close(&self, key: SessionKey) -> Result<(), SessionRejection> {
        if !self.reachable.load(std::sync::atomic::Ordering::SeqCst) {
            let cause = self
                .unreachable_cause
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_default();
            return Err(SessionRejection::Unavailable { cause });
        }
        let mut g = self.inner.write().await;
        let before = g.sessions.len();
        g.sessions
            .retain(|s| !(s.chat_id == key.chat_id && s.thread_id == key.thread_id));
        if g.sessions.len() == before {
            return Err(SessionRejection::UnknownSession {
                key: Self::key_str(&key),
            });
        }
        if g.focused.as_ref() == Some(&key) {
            g.focused = None;
        }
        drop(g);
        self.emit(SessionEvent::Removed {
            chat_id: key.chat_id,
            thread_id: key.thread_id,
        });
        Ok(())
    }

    async fn turns(&self, key: SessionKey, from: u64) -> Result<Vec<TurnEntry>, SessionRejection> {
        let g = self.inner.read().await;
        let Some(sid) = g
            .sessions
            .iter()
            .find(|s| s.chat_id == key.chat_id && s.thread_id == key.thread_id)
            .and_then(|s| s.session_id.clone())
        else {
            return Err(SessionRejection::UnknownSession {
                key: Self::key_str(&key),
            });
        };
        Ok(g.transcripts
            .get(&sid)
            .map(|log| log.iter().filter(|e| e.position >= from).cloned().collect())
            .unwrap_or_default())
    }

    async fn reachability(&self) -> Reachability {
        if self.reachable.load(std::sync::atomic::Ordering::SeqCst) {
            Reachability::Reachable
        } else {
            let cause = self
                .unreachable_cause
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| "核心不可达".into());
            Reachability::Unreachable { cause }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2.2 验收：in-process 满足 trait 且永远 Reachable。
    #[tokio::test]
    async fn in_process_backend_satisfies_trait_and_is_reachable() {
        let map = sebas_router::SessionMap::new();
        let (router, _rx) = sebas_router::RouterHandle::new(map);
        let backend = InProcessBackend::new(router);
        assert_eq!(backend.reachability().await, Reachability::Reachable);
        assert!(backend.snapshot().await.is_empty());
        assert!(backend.focused().await.is_none());
    }

    // 2.3 验收：fake 能驱动每个 trait 方法（无子进程 / socket）。
    #[tokio::test]
    async fn fake_backend_drives_every_trait_method() {
        let backend = FakeBackend::new();
        let mut events = backend.subscribe();

        // spawn → visible + Created event.
        let key = backend.spawn("hi".into(), None).await.unwrap();
        assert_eq!(backend.snapshot().await.len(), 1);
        assert!(matches!(
            events.try_recv(),
            Ok(SessionEvent::Created { .. })
        ));

        // message/close on the key work; unknown keys are rejected.
        assert!(backend.message(key.clone(), "yo".into()).await.is_ok());
        let bogus = SessionKey {
            chat_id: "nope".into(),
            thread_id: None,
        };
        assert_eq!(
            backend.message(bogus.clone(), "yo".into()).await,
            Err(SessionRejection::UnknownSession {
                key: FakeBackend::key_str(&bogus)
            })
        );

        // focus round-trip.
        backend.set_focus(Some(key.clone())).await;
        assert_eq!(backend.focused().await, Some(key.clone()));

        // turns: unknown → rejection; pushed entries filter by position.
        assert!(backend.turns(key.clone(), 0).await.is_err());
        // 给它一个 session_id 再推 transcript。
        backend
            .set_sessions(vec![SessionInfo {
                chat_id: key.chat_id.clone(),
                thread_id: None,
                session_id: Some("s9".into()),
                status: "active".into(),
                phase: None,
                user_prompt: None,
                last_active_unix: 0,
                project_dir: None,
            }])
            .await;
        backend.push_turn("s9", "prompt", "p1").await;
        backend.push_turn("s9", "content", "c1").await;
        let tail = backend.turns(key.clone(), 1).await.unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].position, 1);
        assert_eq!(tail[0].content, "c1");

        // close works and emits Removed.
        assert!(backend.close(key.clone()).await.is_ok());
        assert!(matches!(events.try_recv(), Ok(SessionEvent::Removed { .. })));
        assert!(backend.snapshot().await.is_empty());

        // unreachable mode reports the cause through every mutating path.
        backend.set_reachable(false, "socket absent");
        assert_eq!(
            backend.reachability().await,
            Reachability::Unreachable {
                cause: "socket absent".into()
            }
        );
        assert!(matches!(
            backend.spawn("x".into(), None).await,
            Err(SessionRejection::Unavailable { .. })
        ));
        assert!(matches!(
            backend.message(key, "x".into()).await,
            Err(SessionRejection::Unavailable { .. })
        ));
    }
}
