//! In-process [`SessionBackend`] over the router — the session source for
//! `run --webui`. The core IS this process: every operation goes straight
//! to the [`RouterHandle`], the reachability report is always `Connected`,
//! and the router's `SessionEvent` broadcast is translated into WebUI
//! events for the `/ws` relay.
//!
//! The standalone `sebas webui` subcommand also wires this backend for now
//! (over its own throwaway router) until the socket client lands — that
//! cutover is tasks 7.1/7.2 of add-core-session-channel.

use sebas_feishu::events::SessionKey;
use sebas_router::card_state::CardState;
use sebas_router::router::{RouterHandle, SessionEvent, SessionState};
use sebas_webui::backend::{CloseOutcome, Reachability, Rejection, SessionBackend, TurnContent, TurnItem};
use sebas_webui::events::WebUiEvent;
use sebas_webui::models::{CardConfigInfo, SessionRow, SessionStatus};
use sebas_webui::routes::{card_element_to_view, encode_session_key, format_relative_time};
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct InProcessSessionBackend {
    router: RouterHandle,
    events: broadcast::Sender<WebUiEvent>,
}

impl InProcessSessionBackend {
    /// Wire the backend and start the event pump that forwards the router's
    /// session events to `/ws` clients (translated into WebUI wire events).
    pub fn new(router: RouterHandle) -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        let backend = Arc::new(Self {
            router,
            events: events.clone(),
        });
        let mut rx = backend.router.session_events();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if let Some(ui) = translate_event(event) {
                            // No subscribers is the quiet normal case.
                            let _ = events.send(ui);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            dropped = n,
                            "in-process backend lagged router events; \
                             /ws clients must re-snapshot"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        backend
    }
}

/// Router event → WebUI wire event. The encoded key is the wire identity;
/// an Updated event carries the derived status slug so clients can render
/// the badge without a refetch.
fn translate_event(event: SessionEvent) -> Option<WebUiEvent> {
    match event {
        SessionEvent::Created { session } => Some(WebUiEvent::SessionCreated {
            session_id: encode_session_key(&session.key),
        }),
        SessionEvent::Updated { session } => {
            let state_str = match session.state {
                SessionState::Spawning => "spawning",
                SessionState::Active => "active",
                SessionState::Dormant => "dormant",
            };
            let status = SessionStatus::derive(state_str, &session.phase);
            Some(WebUiEvent::SessionUpdated {
                session_id: encode_session_key(&session.key),
                status: status.slug().to_string(),
            })
        }
        SessionEvent::Removed { key } => Some(WebUiEvent::SessionRemoved {
            session_id: encode_session_key(&key),
        }),
    }
}

#[async_trait::async_trait]
impl SessionBackend for InProcessSessionBackend {
    async fn snapshot(&self) -> Vec<SessionRow> {
        let snapshots = self.router.session_snapshots().await;
        let card_states = self.router.card_state_snapshot().await;
        snapshots
            .into_iter()
            .map(|snapshot| {
                let status: &'static str = match snapshot.state {
                    SessionState::Spawning => "spawning",
                    SessionState::Active => "active",
                    SessionState::Dormant => "dormant",
                };
                let derived = SessionStatus::derive(status, &snapshot.phase);
                // The seed prompt doubles as the detail view's user_prompt.
                let prompt_preview = snapshot
                    .session_id
                    .as_ref()
                    .and_then(|sid| card_states.get(sid))
                    .map(|st: &CardState| st.user_prompt.clone());
                SessionRow {
                    encoded_key: encode_session_key(&snapshot.key),
                    chat_id: snapshot.key.chat_id.clone(),
                    thread_id: snapshot.key.thread_id.clone(),
                    session_id_short: snapshot
                        .session_id
                        .as_deref()
                        .map(|sid| sebas_webui::models::middle_truncate(sid, 18)),
                    session_id: snapshot.session_id.clone(),
                    status,
                    status_label: derived.label(),
                    status_slug: derived.slug(),
                    status_glyph: derived.glyph(),
                    last_active: format_relative_time(snapshot.last_active_unix),
                    last_active_unix: snapshot.last_active_unix,
                    // Focus is a WebUI-side pointer; the API layer applies it.
                    is_active: false,
                    project_dir: snapshot.project_dir,
                    prompt_preview,
                }
            })
            .collect()
    }

    fn subscribe(&self) -> broadcast::Receiver<WebUiEvent> {
        self.events.subscribe()
    }

    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<String, Rejection> {
        let key = self.router.web_spawn(prompt, project_dir).await;
        // web_spawn returns the key even when the placeholder could not be
        // inserted (capacity); verify the mapping exists so a silent
        // failure becomes an honest rejection.
        if self.router.map.get(&key).await.is_none() {
            return Err(Rejection::InvalidRequest {
                reason: "session capacity reached".into(),
            });
        }
        Ok(encode_session_key(&key))
    }

    async fn message(&self, key: &str, message: String) -> Result<(), Rejection> {
        let session_key = decode_or_reject(key)?;
        if self.router.map.get(&session_key).await.is_none() {
            return Err(Rejection::UnknownSession {
                key: key.to_string(),
            });
        }
        self.router.web_send_message(session_key, message).await;
        Ok(())
    }

    async fn close(&self, key: &str) -> Result<CloseOutcome, Rejection> {
        let session_key = decode_or_reject(key)?;
        match self.router.web_close_session(session_key).await {
            sebas_router::router::CloseOutcome::Closed => Ok(CloseOutcome::Closed),
            sebas_router::router::CloseOutcome::NotFound => Ok(CloseOutcome::NotFound),
        }
    }

    async fn turns(&self, key: &str, position: u64) -> Result<TurnContent, Rejection> {
        let session_key = decode_or_reject(key)?;
        let Some(mapping) = self.router.map.get(&session_key).await else {
            return Err(Rejection::UnknownSession {
                key: key.to_string(),
            });
        };
        // A Spawning placeholder has no card state yet: an empty transcript
        // is the honest answer (the detail view renders right after create).
        let Some(session_id) = mapping.session_id().map(str::to_string) else {
            return Ok(TurnContent {
                key: key.to_string(),
                position: 0,
                items: Vec::new(),
            });
        };
        let card_states = self.router.card_state_snapshot().await;
        let items: Vec<TurnItem> = card_states
            .get(&session_id)
            .map(|st: &CardState| {
                st.body
                    .iter()
                    .map(|el| {
                        let view = card_element_to_view(el);
                        TurnItem {
                            kind: view.element_type.to_string(),
                            content: view.content,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        // The returned position is the current head; the items are those
        // after the requested position (a refetch at the head yields none).
        let head = items.len() as u64;
        let items = items.into_iter().skip(position as usize).collect();
        Ok(TurnContent {
            key: key.to_string(),
            position: head,
            items,
        })
    }

    async fn card_config(&self) -> CardConfigInfo {
        let card_cfg = self.router.card_config().await;
        CardConfigInfo {
            theme_color: card_cfg.theme_color,
            fold_long_output: card_cfg.fold_long_output,
            thinking_display: format!("{:?}", card_cfg.thinking),
            max_user_text_chars: card_cfg.max_user_text_chars,
            max_tool_output_chars: card_cfg.max_tool_output_chars,
        }
    }

    /// The core IS this process: always connected.
    fn reachability(&self) -> Reachability {
        Reachability::Connected
    }
}

/// Decode a wire key, mapping a malformed one to `UnknownSession` (the API
/// layer 400-checks keys before calling in; a malformed key here means the
/// session cannot exist).
fn decode_or_reject(key: &str) -> Result<SessionKey, Rejection> {
    sebas_webui::routes::decode_session_key(key).ok_or_else(|| Rejection::UnknownSession {
        key: key.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::InProcessSessionBackend;
    use sebas_router::router::RouterHandle;
    use sebas_router::state::{Mapping, SessionMap};
    use sebas_webui::backend::{Reachability, Rejection, SessionBackend};
    use sebas_webui::events::WebUiEvent;
    use tokio::sync::broadcast;

    /// Wire-form key for a threadless chat id: the NUL-terminated session
    /// key, percent-encoded (what a client can put straight into a URL).
    fn wire(chat: &str) -> String {
        format!("{chat}%00")
    }

    /// Receive the next translated event, tolerating the pump's async hop.
    async fn next_event(rx: &mut broadcast::Receiver<WebUiEvent>) -> WebUiEvent {
        tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("event within 2s")
            .unwrap()
    }

    #[tokio::test]
    async fn reachability_is_unconditionally_connected() {
        let (router, _rx) = RouterHandle::new(SessionMap::new());
        let backend = InProcessSessionBackend::new(router);
        assert_eq!(backend.reachability(), Reachability::Connected);
    }

    /// The snapshot projection: states map to slugs, the seed prompt shows
    /// up as prompt_preview, and focus stays webui-side (is_active false).
    #[tokio::test]
    async fn snapshot_projects_router_state_into_rows() {
        let map = SessionMap::new();
        let key_a = sebas_feishu::events::SessionKey {
            chat_id: "oc_a".into(),
            thread_id: None,
        };
        let key_b = sebas_feishu::events::SessionKey {
            chat_id: "oc_b".into(),
            thread_id: None,
        };
        map.insert(key_a, Mapping::active("s1")).await.unwrap();
        map.insert(key_b, Mapping::dormant("s2", 1)).await.unwrap();

        let (router, _out) = RouterHandle::new(map);
        router.seed_card("s1".into(), "the original prompt".into()).await;
        let backend = InProcessSessionBackend::new(router);

        let rows = backend.snapshot().await;
        assert_eq!(rows.len(), 2, "{rows:?}");
        let a = rows.iter().find(|r| r.encoded_key == wire("oc_a")).unwrap();
        let b = rows.iter().find(|r| r.encoded_key == wire("oc_b")).unwrap();

        assert_eq!(a.status, "active");
        // Active + seed phase ("Get") derives Queued — the honest projection
        // of an idle live session.
        assert_eq!(a.status_slug, "queued");
        assert_eq!(a.session_id.as_deref(), Some("s1"));
        assert_eq!(a.prompt_preview.as_deref(), Some("the original prompt"));
        assert!(!a.is_active, "focus is applied webui-side, not here");

        assert_eq!(b.status, "dormant");
        assert_eq!(b.status_slug, "dormant");
        assert_eq!(b.prompt_preview, None);
    }

    /// Router mutations surface as WebUI events with encoded keys, and the
    /// Updated status slug tracks the phase.
    #[tokio::test]
    async fn router_events_translate_into_webui_events() {
        let (router, _out) = RouterHandle::new(SessionMap::new());
        let backend = InProcessSessionBackend::new(router.clone());
        let mut events = backend.subscribe();

        let key = router.web_spawn("go".into(), None).await;
        let created = next_event(&mut events).await;
        assert!(matches!(
            &created,
            WebUiEvent::SessionCreated { session_id }
                if *session_id == format!("{}\0", key.chat_id).replace('\0', "%00")
        ));

        router.activate(&key, "s9".to_string()).await;
        let updated = next_event(&mut events).await;
        assert!(matches!(
            &updated,
            WebUiEvent::SessionUpdated { status, .. } if status == "queued"
        ));

        router.web_close_session(key).await;
        assert!(matches!(
            next_event(&mut events).await,
            WebUiEvent::SessionRemoved { .. }
        ));
    }

    /// Control operations: spawn returns the encoded key and adds a row;
    /// message/close reject unknown keys instead of silently succeeding.
    #[tokio::test]
    async fn control_operations_reject_unknown_keys() {
        let (router, _out) = RouterHandle::new(SessionMap::new());
        let backend = InProcessSessionBackend::new(router.clone());
        let mut events = backend.subscribe();

        let key = backend.spawn("hi".into(), None).await.unwrap();
        assert!(key.starts_with("web-"), "{key}");
        assert!(key.contains("%00"), "spawned key is the wire form");
        // The spawn's Created event reaches the subscription.
        assert!(matches!(
            next_event(&mut events).await,
            WebUiEvent::SessionCreated { .. }
        ));

        // Known key: accepted.
        backend.message(&key, "do it".into()).await.unwrap();
        // Unknown key: typed rejection, not a silent 200.
        assert_eq!(
            backend.message(&wire("oc_ghost"), "hi".into()).await,
            Err(Rejection::UnknownSession { key: wire("oc_ghost") })
        );

        // Close: known → Closed (row gone from the snapshot), then NotFound.
        assert!(backend.close(&key).await.unwrap() == sebas_webui::backend::CloseOutcome::Closed);
        assert!(matches!(
            next_event(&mut events).await,
            WebUiEvent::SessionRemoved { .. }
        ));
        assert_eq!(backend.snapshot().await.len(), 0);
        assert_eq!(backend.close(&key).await.unwrap(), sebas_webui::backend::CloseOutcome::NotFound);
    }

    /// Turns: a Spawning session serves an empty transcript (the detail view
    /// renders right after create); unknown keys are rejected.
    #[tokio::test]
    async fn turns_serve_empty_for_spawning_and_reject_unknown() {
        let (router, _out) = RouterHandle::new(SessionMap::new());
        let backend = InProcessSessionBackend::new(router);

        let key = backend.spawn("transcript test".into(), None).await.unwrap();
        let content = backend.turns(&key, 0).await.unwrap();
        assert_eq!(content.key, key);
        assert!(content.items.is_empty(), "no card state while spawning");
        assert_eq!(
            backend.turns(&wire("oc_ghost"), 0).await,
            Err(Rejection::UnknownSession { key: wire("oc_ghost") })
        );
    }
}
