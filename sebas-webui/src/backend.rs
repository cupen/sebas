//! The session backend seam.
//!
//! The WebUI crate must not know whether the session core lives in this
//! process ([`RouterHandle`]-backed) or across a Unix socket (standalone
//! `sebas webui` talking to the daemon). Everything session-shaped flows
//! through [`SessionBackend`]:
//!
//! - [`SessionBackend::snapshot`] — every known session, row-ready;
//! - [`SessionBackend::subscribe`] — live `WebUiEvent`s for the `/ws` relay;
//! - [`SessionBackend::spawn`] / [`message`] / [`close`] — control, with
//!   typed [`Rejection`]s the routes map onto HTTP status codes;
//! - [`SessionBackend::turns`] — accumulated turn content addressed by a
//!   monotonic position, so a client fetches only what it has not seen;
//! - [`SessionBackend::reachability`] — the honest-degradation report the
//!   summary endpoint surfaces as `core_connected` (+ cause).
//!
//! [`RouterHandle`]: ../sebas_router/router/struct.RouterHandle.html
//! [`message`]: SessionBackend::message

use crate::events::WebUiEvent;
use crate::models::{CardConfigInfo, SessionRow};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{broadcast, RwLock};

/// Why a control operation could not be performed. Routes map these onto
/// existing status codes (`UnknownSession` → 404) and the SPA renders the
/// message verbatim — it must never leak paths or internals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum Rejection {
    /// No session exists for the given key (stale URL, already closed).
    UnknownSession { key: String },
    /// The core is not connected; nothing was mutated.
    CoreUnreachable { cause: String },
    /// The request itself is malformed or refused (e.g. capacity).
    InvalidRequest { reason: String },
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSession { key } => write!(f, "unknown session: {key}"),
            Self::CoreUnreachable { cause } => write!(f, "core unreachable: {cause}"),
            Self::InvalidRequest { reason } => write!(f, "{reason}"),
        }
    }
}

/// Outcome of a close request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseOutcome {
    /// The session existed and was torn down.
    Closed,
    /// No mapping existed (already closed, or stale URL).
    NotFound,
}

/// How healthy the connection to the session core is. `Unreachable` carries
/// a human-readable cause — socket absent, refused, secret rejected, or the
/// stream was dropped — that the UI shows verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Reachability {
    Connected,
    Unreachable { cause: String },
}

/// One item of a session's accumulated turn content (a markdown block, an
/// error line, a tool trace — whatever the core accumulated for the card).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnItem {
    /// Element kind, e.g. `"text"` or `"error"`.
    pub kind: String,
    pub content: String,
}

/// A slice of turn content. `position` is the index of the LAST item
/// included; pass it back to receive only newer items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnContent {
    pub key: String,
    pub position: u64,
    pub items: Vec<TurnItem>,
}

/// The seam every session source implements. Implementations live in the
/// sebas binary crate (in-process over the router, socket client to the
/// core); tests use [`FakeBackend`].
#[async_trait]
pub trait SessionBackend: Send + Sync + 'static {
    /// Every known session, ready to render (rows carry derived status and
    /// encoded keys). Most-recent-activity first.
    async fn snapshot(&self) -> Vec<SessionRow>;

    /// Subscribe to live session events. A lagging receiver gets
    /// `RecvError::Lagged` and must re-snapshot — events are a notification,
    /// never a gap-free log.
    fn subscribe(&self) -> broadcast::Receiver<WebUiEvent>;

    /// Spawn a new web-originated session. Returns the encoded key of the
    /// (immediately visible, starting) session.
    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<String, Rejection>;

    /// Send a user message to an existing session.
    async fn message(&self, key: &str, message: String) -> Result<(), Rejection>;

    /// Tear a session down.
    async fn close(&self, key: &str) -> Result<CloseOutcome, Rejection>;

    /// Turn content for a session after `position`.
    async fn turns(&self, key: &str, position: u64) -> Result<TurnContent, Rejection>;

    /// The core's card-rendering config, for the settings page. Required by
    /// the webui-api spec ("GET /api/settings carries card config + gateway
    /// info") — read-only display data; the channel does not stream config
    /// changes, so a socket backend serves its last-known snapshot.
    async fn card_config(&self) -> CardConfigInfo;

    /// The honest-degradation report (drives `core_connected` in the API).
    fn reachability(&self) -> Reachability;
}

/// A test double: no child processes, no sockets, fully scriptable. Sessions
/// are whatever rows you set; `unreachable` mode fails every control
/// operation with [`Rejection::CoreUnreachable`]; [`FakeBackend::emit`]
/// pushes events through the same broadcast the `/ws` relay consumes.
pub struct FakeBackend {
    state: RwLock<FakeState>,
    events: broadcast::Sender<WebUiEvent>,
    reachable: AtomicBool,
    card_config: RwLock<CardConfigInfo>,
}

#[derive(Default)]
struct FakeState {
    rows: Vec<SessionRow>,
    turns: HashMap<String, Vec<TurnItem>>,
    spawn_seq: u64,
    messages: Vec<(String, String)>,
    closes: Vec<String>,
    spawner_calls: Vec<(String, Option<String>)>,
}

impl FakeBackend {
    pub fn new(reachable: bool) -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            state: RwLock::new(FakeState::default()),
            events,
            reachable: AtomicBool::new(reachable),
            card_config: RwLock::new(CardConfigInfo::default()),
        }
    }

    pub fn connected() -> Self {
        Self::new(true)
    }

    pub fn unreachable() -> Self {
        Self::new(false)
    }

    pub async fn set_rows(&self, rows: Vec<SessionRow>) {
        self.state.write().await.rows = rows;
    }

    /// Set the card config the settings endpoint serves.
    pub async fn set_card_config(&self, cfg: CardConfigInfo) {
        *self.card_config.write().await = cfg;
    }

    /// Append turn content for `key`; [`SessionBackend::turns`] slices it.
    pub async fn push_turn(&self, key: &str, item: TurnItem) {
        self.state.write().await.turns.entry(key.to_string()).or_default().push(item);
    }

    pub fn set_reachable(&self, reachable: bool) {
        self.reachable.store(reachable, Ordering::SeqCst);
    }

    /// Push an event as if the core had published it.
    pub fn emit(&self, event: WebUiEvent) {
        let _ = self.events.send(event);
    }

    /// Recorded `message` calls, for assertions.
    pub async fn messages(&self) -> Vec<(String, String)> {
        self.state.read().await.messages.clone()
    }

    /// Recorded `close` calls, for assertions.
    pub async fn closes(&self) -> Vec<String> {
        self.state.read().await.closes.clone()
    }

    /// Recorded `spawn` calls, for assertions.
    pub async fn spawn_calls(&self) -> Vec<(String, Option<String>)> {
        self.state.read().await.spawner_calls.clone()
    }
}

#[async_trait]
impl SessionBackend for FakeBackend {
    async fn snapshot(&self) -> Vec<SessionRow> {
        self.state.read().await.rows.clone()
    }

    fn subscribe(&self) -> broadcast::Receiver<WebUiEvent> {
        self.events.subscribe()
    }

    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<String, Rejection> {
        if !self.reachable.load(Ordering::SeqCst) {
            return Err(Rejection::CoreUnreachable {
                cause: "not connected to the session core".into(),
            });
        }
        let mut state = self.state.write().await;
        state.spawn_seq += 1;
        // The contract returns the encoded wire key (what a client can put
        // straight into a URL), matching the real backends.
        let key = urlencoding::encode(&format!("web-{}\0", state.spawn_seq)).into_owned();
        state.rows.push(SessionRow {
            encoded_key: key.clone(),
            chat_id: key.clone(),
            thread_id: None,
            session_id: None,
            session_id_short: None,
            status: "spawning",
            status_label: "Starting",
            status_slug: "starting",
            status_glyph: "◇",
            last_active: "now".into(),
            last_active_unix: 0,
            is_active: false,
            project_dir: project_dir.clone(),
            prompt_preview: Some(prompt.clone()),
        });
        state.spawner_calls.push((prompt, project_dir));
        // Live event, as the core would publish on session creation.
        let _ = self.events.send(WebUiEvent::SessionCreated {
            session_id: key.clone(),
        });
        Ok(key)
    }

    async fn message(&self, key: &str, message: String) -> Result<(), Rejection> {
        if !self.reachable.load(Ordering::SeqCst) {
            return Err(Rejection::CoreUnreachable {
                cause: "not connected to the session core".into(),
            });
        }
        let mut state = self.state.write().await;
        let known = state.rows.iter().any(|r| r.encoded_key == key);
        if !known {
            return Err(Rejection::UnknownSession { key: key.to_string() });
        }
        state.messages.push((key.to_string(), message));
        Ok(())
    }

    async fn close(&self, key: &str) -> Result<CloseOutcome, Rejection> {
        if !self.reachable.load(Ordering::SeqCst) {
            return Err(Rejection::CoreUnreachable {
                cause: "not connected to the session core".into(),
            });
        }
        let mut state = self.state.write().await;
        let before = state.rows.len();
        state.rows.retain(|r| r.encoded_key != key);
        state.closes.push(key.to_string());
        if state.rows.len() == before {
            Ok(CloseOutcome::NotFound)
        } else {
            // Live event, as the core would publish on teardown.
            let _ = self.events.send(WebUiEvent::SessionRemoved {
                session_id: key.to_string(),
            });
            Ok(CloseOutcome::Closed)
        }
    }

    async fn turns(&self, key: &str, position: u64) -> Result<TurnContent, Rejection> {
        let state = self.state.read().await;
        // A known session without registered turns serves an empty
        // transcript (a spawning session has no card state yet) — only an
        // unknown key is a typed rejection.
        let Some(items) = state.turns.get(key) else {
            if state.rows.iter().any(|r| r.encoded_key == key) {
                return Ok(TurnContent {
                    key: key.to_string(),
                    position: 0,
                    items: Vec::new(),
                });
            }
            return Err(Rejection::UnknownSession { key: key.to_string() });
        };
        let slice: Vec<TurnItem> = items
            .iter()
            .skip(position as usize)
            .cloned()
            .collect();
        Ok(TurnContent {
            key: key.to_string(),
            position: items.len() as u64,
            items: slice,
        })
    }

    async fn card_config(&self) -> CardConfigInfo {
        self.card_config.read().await.clone()
    }

    fn reachability(&self) -> Reachability {
        if self.reachable.load(Ordering::SeqCst) {
            Reachability::Connected
        } else {
            Reachability::Unreachable {
                cause: "not connected to the session core".into(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CloseOutcome, FakeBackend, Reachability, Rejection, SessionBackend, TurnItem};
    use crate::events::WebUiEvent;
    use crate::models::{CardConfigInfo, SessionRow};
    use std::sync::Arc;

    fn row(key: &str) -> SessionRow {
        SessionRow {
            encoded_key: key.into(),
            chat_id: key.into(),
            thread_id: None,
            session_id: Some("ses_1".into()),
            session_id_short: Some("ses_1".into()),
            status: "active",
            status_label: "Working",
            status_slug: "working",
            status_glyph: "▶",
            last_active: "just now".into(),
            last_active_unix: 42,
            is_active: false,
            project_dir: None,
            prompt_preview: None,
        }
    }

    /// Task 2.3: every trait method drives without a child process or socket,
    /// including the unreachable mode and the event channel.
    #[tokio::test]
    async fn fake_backend_drives_every_method() {
        let backend = Arc::new(FakeBackend::connected());
        let mut events = backend.subscribe();

        // snapshot: initially empty, then whatever was set.
        assert!(backend.snapshot().await.is_empty());
        backend.set_rows(vec![row("oc_chat"), row("oc_seed")]).await;
        let snapshot = backend.snapshot().await;
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].encoded_key, "oc_chat");

        // subscribe + emit: the event crosses the broadcast.
        backend.emit(WebUiEvent::SessionCreated {
            session_id: "web-2".into(),
        });
        let got = events.try_recv().unwrap();
        assert!(matches!(got, WebUiEvent::SessionCreated { ref session_id } if session_id == "web-2"));

        // spawn: returns a fresh encoded wire key and grows the snapshot.
        let spawned = backend
            .spawn("fix the bug".into(), Some("/tmp/proj".into()))
            .await
            .unwrap();
        assert_eq!(spawned, "web-1%00", "spawn returns the encoded wire key");
        assert_eq!(backend.snapshot().await.len(), 3);
        assert_eq!(
            backend.spawn_calls().await,
            vec![("fix the bug".to_string(), Some("/tmp/proj".to_string()))]
        );

        // message: known key records; unknown key rejects.
        backend.message(&spawned, "do it".into()).await.unwrap();
        assert_eq!(
            backend.messages().await,
            vec![(spawned.clone(), "do it".to_string())]
        );
        assert_eq!(
            backend.message("nope", "hi".into()).await,
            Err(Rejection::UnknownSession { key: "nope".into() })
        );

        // close: known → Closed (row gone); unknown → NotFound.
        assert_eq!(backend.close(&spawned).await.unwrap(), CloseOutcome::Closed);
        assert_eq!(backend.snapshot().await.len(), 2);
        assert_eq!(backend.close(&spawned).await.unwrap(), CloseOutcome::NotFound);

        // turns: monotonic position — a second call at the returned position
        // yields only newer content.
        backend
            .push_turn("oc_chat", TurnItem { kind: "text".into(), content: "one".into() })
            .await;
        backend
            .push_turn("oc_chat", TurnItem { kind: "error".into(), content: "boom".into() })
            .await;
        let first = backend.turns("oc_chat", 0).await.unwrap();
        assert_eq!(first.items.len(), 2);
        assert_eq!(first.position, 2);
        let second = backend.turns("oc_chat", first.position).await.unwrap();
        assert!(second.items.is_empty(), "no repeats at the same position");
        backend
            .push_turn("oc_chat", TurnItem { kind: "text".into(), content: "three".into() })
            .await;
        let third = backend.turns("oc_chat", second.position).await.unwrap();
        assert_eq!(third.items.len(), 1);
        assert_eq!(third.items[0].content, "three");
        assert_eq!(
            backend.turns("ghost", 0).await,
            Err(Rejection::UnknownSession { key: "ghost".into() })
        );

        // card_config: the settings payload's card-config source.
        let cfg = backend.card_config().await;
        assert!(!cfg.theme_color.is_empty(), "default theme color present");
        backend
            .set_card_config(CardConfigInfo {
                theme_color: "#ff0000".into(),
                ..CardConfigInfo::default()
            })
            .await;
        assert_eq!(backend.card_config().await.theme_color, "#ff0000");

        // reachability: connected by default.
        assert_eq!(backend.reachability(), Reachability::Connected);
    }

    /// Unreachable mode: every control op fails with CoreUnreachable and
    /// nothing mutates.
    #[tokio::test]
    async fn unreachable_mode_fails_control_without_mutating() {
        let backend = FakeBackend::unreachable();
        backend.set_rows(vec![row("oc_x")]).await;

        assert_eq!(
            backend.reachability(),
            Reachability::Unreachable { cause: "not connected to the session core".into() }
        );
        let cause_err = backend.spawn("p".into(), None).await.unwrap_err();
        assert!(matches!(cause_err, Rejection::CoreUnreachable { .. }));
        assert!(backend.message("oc_x", "m".into()).await.is_err());
        assert!(backend.close("oc_x").await.is_err());
        // Nothing mutated: no spawn rows appeared, message/close unrecorded.
        assert_eq!(backend.snapshot().await.len(), 1);
        assert!(backend.messages().await.is_empty());
        assert!(backend.closes().await.is_empty());
        // The rejections serialize with a tagged shape for the JSON API.
        let json = serde_json::to_value(&cause_err).unwrap();
        assert_eq!(json["error"], "core_unreachable");
        // Reading still works while unreachable (rows come from the last
        // known snapshot).
        assert_eq!(backend.snapshot().await[0].encoded_key, "oc_x");
    }

    /// The trait object is what the state holds — verify it dispatches.
    #[tokio::test]
    async fn usable_as_trait_object() {
        let fake = Arc::new(FakeBackend::connected());
        let backend: Arc<dyn SessionBackend> = fake.clone();
        fake.set_rows(vec![row("k")]).await;
        assert_eq!(backend.snapshot().await.len(), 1);
        assert_eq!(backend.reachability(), Reachability::Connected);
    }
}
