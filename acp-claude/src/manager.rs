use crate::client::{AcpClient, SpawnConfig};
use crate::session::{AcpCommand, AcpEvent, SessionMeta};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SessionManager {
    inner: Arc<Mutex<HashMap<String, SessionMeta>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn create_session(
        &self,
        claude_path: &str,
        args: Vec<String>,
        work_dir: Option<String>,
        _prompt: String,
    ) -> anyhow::Result<String> {
        let cfg = SpawnConfig {
            claude_path: claude_path.to_string(),
            claude_args: args,
            work_dir,
        };
        let session_id = uuid::Uuid::new_v4().to_string();
        let handle = AcpClient::spawn(&cfg)?;
        self.inner.lock().await.insert(
            session_id.clone(),
            SessionMeta {
                session_id: session_id.clone(),
                handle,
            },
        );
        Ok(session_id)
    }

    pub async fn kill(&self, session_id: &str) {
        let meta = self.inner.lock().await.remove(session_id);
        if let Some(m) = meta {
            drop(m.handle.cmd_tx); // closing tx causes stdin task to exit
        }
    }

    pub async fn send(&self, session_id: &str, cmd: AcpCommand) -> anyhow::Result<()> {
        let g = self.inner.lock().await;
        let m = g
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("unknown session"))?;
        m.handle.cmd_tx.send(cmd).await?;
        Ok(())
    }

    pub async fn next_event(&self, session_id: &str) -> Option<AcpEvent> {
        let g = self.inner.lock().await;
        let m = g.get(session_id)?;
        let mut rx = m.handle.evt_rx.lock().await;
        rx.recv().await
    }

    /// Clone the per-session event receiver handle. Unlike [`next_event`], this
    /// only holds the manager-wide lock long enough to clone the `Arc`, so a
    /// long-lived event pump can `recv()` without blocking `create_session` /
    /// `send` on other sessions.
    pub async fn event_rx(
        &self,
        session_id: &str,
    ) -> Option<Arc<Mutex<tokio::sync::mpsc::Receiver<AcpEvent>>>> {
        let g = self.inner.lock().await;
        g.get(session_id).map(|m| m.handle.evt_rx.clone())
    }

    /// Cancel and drop every live session. Called on daemon shutdown so child
    /// processes are signalled before the state snapshot is written.
    pub async fn kill_all(&self) {
        let metas: Vec<SessionMeta> =
            self.inner.lock().await.drain().map(|(_, m)| m).collect();
        for m in metas {
            let _ = m
                .handle
                .cmd_tx
                .send(AcpCommand::Cancel {
                    session_id: m.session_id.clone(),
                })
                .await;
            // Dropping `m` closes cmd_tx (its stdin task exits) and drops the
            // Child, which was spawned with kill_on_drop(true).
        }
    }
}