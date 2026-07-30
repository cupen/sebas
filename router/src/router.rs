use crate::commands::{parse_command, Command};
use crate::state::SessionMap;
use acp_claude::session::{AcpCommand, AcpEvent, Decision};
use feishu::cards::{
    apply_event, render_dead_session_card, render_permission_card, render_root_card,
};
use feishu::events::{CardAction, FeishuIn, SessionKey};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, RwLock};

#[derive(Debug)]
pub enum Out {
    SpawnAcp {
        key: SessionKey,
        prompt: String,
    },
    SendAcp {
        session_id: String,
        cmd: AcpCommand,
    },
    SendCard {
        key: SessionKey,
        card: serde_json::Value,
        msg_id: Option<String>,
    },
    UpdateCard {
        session_id: String,
        card: serde_json::Value,
    },
    React {
        session_id: String,
        emoji: String,
    },
    HelpText {
        key: SessionKey,
    },
}

pub struct RouterHandle {
    map: SessionMap,
    tx: mpsc::Sender<Out>,
    msgid: MsgIdMap,
}

impl Clone for RouterHandle {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
            tx: self.tx.clone(),
            msgid: self.msgid.clone(),
        }
    }
}

impl RouterHandle {
    pub fn new(map: SessionMap) -> (Self, mpsc::Receiver<Out>) {
        let (tx, rx) = mpsc::channel(256);
        (
            Self {
                map,
                tx,
                msgid: MsgIdMap::default(),
            },
            rx,
        )
    }

    pub async fn dump_json(&self) -> serde_json::Result<String> {
        self.map.dump_json().await
    }

    /// Record the root card message_id for a session. Called from the outbound
    /// pump after the first `send_card` returns its message_id.
    pub async fn record_root_msg_id(&self, session_id: String, msg_id: String) {
        self.msgid.record(session_id, msg_id).await;
    }

    /// Look up the root card message_id for a session (used by `UpdateCard`).
    pub async fn root_msg_id(&self, session_id: &str) -> Option<String> {
        self.msgid.get(session_id).await
    }

    /// Record a `SessionKey -> session_id` mapping. Called by the dispatcher
    /// once `SessionManager::create_session` has minted the real session_id, so
    /// that continuations, permission-card routing (reverse lookup) and
    /// liveness checks can find the session.
    pub async fn insert_mapping(&self, key: SessionKey, session_id: String) {
        let mapping = crate::state::Mapping {
            session_id,
            last_active_unix: now_unix(),
        };
        if let Err(e) = self.map.insert(key, mapping).await {
            tracing::warn!(?e, "failed to insert session mapping");
        }
    }

    /// True if a live session is mapped for `key` (used to reject button
    /// callbacks that arrive after a session has ended).
    pub async fn session_alive(&self, key: &SessionKey) -> bool {
        self.map.get(key).await.is_some()
    }

    pub async fn dispatch(&self, evt: FeishuIn) {
        match evt {
            FeishuIn::Text { key, text, .. } => self.on_text(key, text).await,
            FeishuIn::Media {
                key,
                files,
                caption,
            } => {
                let prompt = compose_media_prompt(&text_from_caption(&caption), &files);
                self.on_text(key, prompt).await;
            }
            FeishuIn::ButtonCb { key, action } => self.on_button(key, action).await,
        }
    }

    /// Dispatch an inbound `AcpEvent`, extracting the session_id from the
    /// event payload and forwarding to `apply_event_to_out`.
    pub async fn dispatch_acp_event(&self, event: AcpEvent) {
        let session_id = extract_session_id(&event).to_owned();
        self.apply_event_to_out(session_id, &event).await;
    }

    pub async fn apply_event_to_out(&self, session_id: String, event: &AcpEvent) {
        match event {
            AcpEvent::TextDelta { .. }
            | AcpEvent::ToolStart { .. }
            | AcpEvent::Finished { .. }
            | AcpEvent::Error { .. } => {
                let emoji = if matches!(event, AcpEvent::Finished { .. }) {
                    "✅"
                } else {
                    "🚧"
                };
                let mut card = render_root_card("", &session_id, emoji);
                apply_event(&mut card, event);
                let _ = self
                    .tx
                    .send(Out::UpdateCard {
                        session_id,
                        card: serde_json::to_value(&card).unwrap(),
                    })
                    .await;
            }
            AcpEvent::PermissionRequest {
                session_id,
                request_id,
                tool_name,
                args,
            } => {
                let card = render_permission_card(session_id, request_id, tool_name, args);
                // Resolve the SessionKey that owns this session so Feishu has a
                // real `receive_id`. Without this the card would carry an empty
                // chat_id and Feishu rejects it.
                let Some(key) = self.map.lookup_key_by_session(session_id).await else {
                    tracing::warn!(%session_id, "no SessionKey for permission request; dropping card");
                    return;
                };
                let _ = self
                    .tx
                    .send(Out::SendCard {
                        key,
                        card: serde_json::to_value(&card).unwrap(),
                        msg_id: None,
                    })
                    .await;
            }
            _ => {}
        }
    }

    async fn on_text(&self, key: SessionKey, text: String) {
        match parse_command(&text) {
            Command::New => self.spawn_new(key, String::new()).await,
            Command::Help => {
                let _ = self.tx.send(Out::HelpText { key }).await;
            }
            Command::PassThrough(p) => {
                if let Some(m) = self.map.get(&key).await {
                    self.continue_session(m.session_id, p).await;
                } else {
                    self.spawn_new(key, p).await;
                }
            }
            Command::Compact | Command::Cost | Command::Cancel | Command::Status => {
                if let Some(m) = self.map.get(&key).await {
                    self.forward_to_session(&m.session_id, text).await;
                } else {
                    let _ = self.tx.send(Out::HelpText { key }).await;
                }
            }
            _ => {
                let _ = self.tx.send(Out::HelpText { key }).await;
            }
        }
    }

    async fn on_button(&self, key: SessionKey, action: CardAction) {
        // If the session is gone (process exited / daemon restarted), the
        // permission reply has nowhere to go — tell the user instead of sending
        // a command into the void.
        if !self.session_alive(&key).await {
            let card = render_dead_session_card();
            let _ = self
                .tx
                .send(Out::SendCard {
                    key,
                    card: serde_json::to_value(&card).unwrap(),
                    msg_id: None,
                })
                .await;
            return;
        }
        let decision = match action.decision.as_deref() {
            Some("allow_once") => Decision::AllowOnce,
            Some("allow_session") => Decision::AllowSession,
            // Fail closed: unknown or missing decision is a deny.
            _ => Decision::Deny,
        };
        match (action.session_id.clone(), action.request_id.clone()) {
            (sid, Some(rid)) => {
                let _ = self
                    .tx
                    .send(Out::SendAcp {
                        session_id: sid.clone(),
                        cmd: AcpCommand::PermissionReply {
                            session_id: sid,
                            request_id: rid,
                            decision,
                        },
                    })
                    .await;
            }
            _ => {
                let _ = self.tx.send(Out::HelpText { key }).await;
            }
        }
    }

    async fn spawn_new(&self, key: SessionKey, prompt: String) {
        // Only emit SpawnAcp. The root card is sent by the dispatcher *after*
        // `create_session` mints the real session_id, so the card's MsgIdMap
        // entry (and later streaming UpdateCards) key off that session_id.
        let _ = self.tx.send(Out::SpawnAcp { key, prompt }).await;
    }

    async fn continue_session(&self, session_id: String, prompt: String) {
        let _ = self
            .tx
            .send(Out::SendAcp {
                session_id: session_id.clone(),
                cmd: AcpCommand::ContinueSession { session_id, prompt },
            })
            .await;
    }

    async fn forward_to_session(&self, session_id: &str, text: String) {
        let cmd = match parse_command(&text) {
            Command::Compact => AcpCommand::ContinueSession {
                session_id: session_id.into(),
                prompt: "/compact".into(),
            },
            Command::Cost => AcpCommand::ContinueSession {
                session_id: session_id.into(),
                prompt: "/cost".into(),
            },
            Command::Cancel => AcpCommand::Cancel {
                session_id: session_id.into(),
            },
            _ => return,
        };
        let _ = self
            .tx
            .send(Out::SendAcp {
                session_id: session_id.into(),
                cmd,
            })
            .await;
    }
}

pub fn compose_media_prompt(caption: &str, files: &[String]) -> String {
    let mut out = String::new();
    if !caption.is_empty() {
        out.push_str(caption);
        out.push('\n');
    }
    out.push_str("\n[attached: ");
    out.push_str(&files.join(", "));
    out.push(']');
    out
}

fn text_from_caption(c: &Option<String>) -> String {
    c.clone().unwrap_or_default()
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn extract_session_id(event: &AcpEvent) -> &str {
    match event {
        AcpEvent::TextDelta { session_id, .. }
        | AcpEvent::ThinkingDelta { session_id, .. }
        | AcpEvent::ToolStart { session_id, .. }
        | AcpEvent::ToolProgress { session_id, .. }
        | AcpEvent::ToolEnd { session_id, .. }
        | AcpEvent::PermissionRequest { session_id, .. }
        | AcpEvent::Finished { session_id }
        | AcpEvent::Error { session_id, .. } => session_id,
    }
}

/// Tracks root-card message_ids per session so `UpdateCard` can resolve a
/// `session_id` to a `message_id` (Feishu's PATCH endpoint needs the
/// message_id, not the session_id).
#[derive(Default, Clone)]
pub struct MsgIdMap {
    inner: Arc<RwLock<HashMap<String, String>>>,
}

impl MsgIdMap {
    pub async fn record(&self, session_id: String, msg_id: String) {
        self.inner.write().await.insert(session_id, msg_id);
    }

    pub async fn get(&self, session_id: &str) -> Option<String> {
        self.inner.read().await.get(session_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn msgid_record_and_get_round_trip() {
        let m = MsgIdMap::default();
        assert!(m.get("s1").await.is_none());
        m.record("s1".into(), "om_abc".into()).await;
        assert_eq!(m.get("s1").await.as_deref(), Some("om_abc"));
        // overwrite
        m.record("s1".into(), "om_def".into()).await;
        assert_eq!(m.get("s1").await.as_deref(), Some("om_def"));
        // isolation
        m.record("s2".into(), "om_xyz".into()).await;
        assert_eq!(m.get("s2").await.as_deref(), Some("om_xyz"));
    }
}
