//! `SessionManager` — public API for spawning per-session claude agents.
//!
//! Engine: `cc-agent-sdk` over claude's stream-json + control protocol
//! (post-ACP; see docs/superpowers/specs/2026-08-06-claude-direct-sdk-refactor-design.md).
//! The public surface (`SessionManager`, `SessionStart`, `SpawnOutcome`,
//! `AcpCommand`/`AcpEvent` in session.rs) is unchanged from the ACP era —
//! router/run.rs and the test-suite are the proof of that.

use crate::driver::{CcDriver, ConnectConfig};
use crate::session::{AcpCommand, AcpEvent, AcpSessionHandle, SessionMeta};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};

/// How the background session task should establish the claude session.
#[derive(Debug, Clone)]
pub enum SessionStart {
    /// Fresh conversation (a uuid is minted and passed as `--session-id`).
    New,
    /// `resume` the given (previously persisted) conversation id —
    /// claude-native resume; the routing id stays the same (spec §3.3e).
    Load(String),
}

/// What `spawn` actually established. `session_id` is always the id to
/// route by: for a successful `Load` it is the resumed conversation id;
/// after a resume-rejection fallback it is the freshly minted id.
#[derive(Debug, Clone)]
pub struct SpawnOutcome {
    pub session_id: String,
    /// True only when a `Load` actually resumed the old conversation;
    /// false means either a fresh spawn or a resume-rejection fallback
    /// (old conversation gone — the caller should tell the user).
    pub resumed: bool,
}

pub struct SessionManager {
    inner: Arc<Mutex<HashMap<String, SessionMeta>>>,
    startup_timeout: std::time::Duration,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new(std::time::Duration::from_secs(30))
    }
}

impl SessionManager {
    pub fn new(startup_timeout: std::time::Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            startup_timeout,
        }
    }

    /// `prompt` is part of the public API surface but is **not** sent here —
    /// the prompt is forwarded exactly once via `AcpCommand::CreateSession`
    /// (or `ContinueSession`) through `send`. (Unchanged contract.)
    pub async fn create_session(
        &self,
        path: &str,
        args: Vec<String>,
        work_dir: Option<String>,
        _prompt: String,
    ) -> anyhow::Result<String> {
        Ok(self
            .spawn(path, args, work_dir, SessionStart::New)
            .await?
            .session_id)
    }

    /// Lazily respawn a previously persisted session (spec §3.3e): spawn
    /// claude with `resume = old_session_id`. The routing id IS the resumed
    /// conversation id. Graceful fallback (sebas-dk8.4): if claude rejects
    /// the resume (conversation files gone — "No conversation found"), a
    /// fresh session is started transparently with a NEW id and
    /// `SpawnOutcome.resumed == false` tells the caller to inform the user
    /// that the old conversation is gone.
    pub async fn resume_session(
        &self,
        path: &str,
        args: Vec<String>,
        work_dir: Option<String>,
        old_session_id: &str,
    ) -> anyhow::Result<SpawnOutcome> {
        self.spawn(
            path,
            args,
            work_dir,
            SessionStart::Load(old_session_id.to_string()),
        )
        .await
    }

    /// Spawn a per-session claude subprocess, complete the SDK initialize
    /// handshake, and start the driver's read/command loop. Returns the id
    /// callers use as the routing key for `send`/`event_rx`/`kill`.
    async fn spawn(
        &self,
        path: &str,
        args: Vec<String>,
        work_dir: Option<String>,
        start: SessionStart,
    ) -> anyhow::Result<SpawnOutcome> {
        let (session_id, resume) = match &start {
            SessionStart::New => (uuid::Uuid::new_v4().to_string(), false),
            SessionStart::Load(old) => (old.clone(), true),
        };

        let (cmd_tx, cmd_rx) = mpsc::channel::<AcpCommand>(64);
        let (evt_tx, evt_rx) = mpsc::channel::<AcpEvent>(256);
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let pending_responders: Arc<Mutex<HashMap<String, crate::session::ResponderSlot>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let expected_exit = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let terminal_sent = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let make_config = |sid: String, resume: bool| ConnectConfig {
            claude_path: path.to_string(),
            claude_args: args.clone(),
            work_dir: work_dir.clone(),
            session_id: sid,
            resume,
            startup_timeout: self.startup_timeout,
            evt_tx: evt_tx.clone(),
            pending_perms: pending_responders.clone(),
            terminal_sent: terminal_sent.clone(),
        };

        let (driver, session_id, resumed) =
            match CcDriver::connect(make_config(session_id.clone(), resume)).await {
                Ok(d) => (d, session_id, resume),
                Err(crate::driver::ConnectError::ResumeRejected) => {
                    // Graceful fallback (sebas-dk8.4): the old conversation is
                    // gone (claude's session files were cleaned). Start fresh
                    // with a NEW id instead of failing the spawn; `resumed:
                    // false` tells run.rs to show the user a notice.
                    let fresh = uuid::Uuid::new_v4().to_string();
                    tracing::warn!(
                        old = %session_id,
                        fresh = %fresh,
                        "claude rejected resume; falling back to a fresh session"
                    );
                    let d = CcDriver::connect(make_config(fresh.clone(), false)).await?;
                    (d, fresh, false)
                }
                Err(crate::driver::ConnectError::Other(e)) => return Err(e),
            };

        tokio::spawn({
            let inner = self.inner.clone();
            let sid = session_id.clone();
            let expected_exit = expected_exit.clone();
            async move {
                driver.run(cmd_rx, cancel_rx).await;
                // Terminal-event guarantee (design §4.2): a session that dies
                // without an explicit kill surfaces exactly one
                // Error{terminal:true}; then the table entry is dropped so
                // send() fails fast and all senders close the stream.
                //
                // Eager removal is safe: the run.rs pump (and the crash tests)
                // clone `Arc<Mutex<Receiver>>` via `event_rx()` BEFORE the
                // session can die, so any buffered terminal event survives
                // map removal — the dropped entry only releases the manager's
                // Arc clone, not the consumer's.
                if !expected_exit.load(std::sync::atomic::Ordering::SeqCst)
                    && !terminal_sent.load(std::sync::atomic::Ordering::SeqCst)
                {
                    let _ = evt_tx
                        .send(AcpEvent::Error {
                            session_id: sid.clone(),
                            message: "agent process exited".into(),
                            terminal: true,
                        })
                        .await;
                }
                // ALWAYS remove the entry — upholds the "manager 表无残留"
                // invariant. `kill()` already removed → no-op here.
                inner.lock().await.remove(&sid);
            }
        });

        let handle = AcpSessionHandle {
            session_id: session_id.clone(),
            cmd_tx,
            evt_rx: Arc::new(Mutex::new(evt_rx)),
            cancel_tx: Some(cancel_tx),
            pending_responders,
        };
        self.inner.lock().await.insert(
            session_id.clone(),
            SessionMeta {
                session_id: session_id.clone(),
                handle,
                expected_exit,
            },
        );

        Ok(SpawnOutcome {
            session_id,
            resumed,
        })
    }

    /// Stop one session. Drops the cancel sender; the driver loop exits and
    /// the child is SIGKILLed on disconnect/drop.
    pub async fn kill(&self, session_id: &str) {
        if let Some(meta) = self.inner.lock().await.remove(session_id) {
            meta.expected_exit
                .store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(tx) = meta.handle.cancel_tx {
                let _ = tx.send(());
            }
        }
    }

    /// Dispatch an `AcpCommand`.
    ///
    /// - `PermissionReply` resolves the oneshot parked by the driver's
    ///   PreToolUse hook callback. Does not touch the command channel.
    /// - `Cancel` cancels the current turn; the driver heals the session by
    ///   respawning with `resume` (D4 semantics under the new engine).
    /// - Everything else (CreateSession, ContinueSession) is forwarded to
    ///   the command channel for the driver to handle.
    pub async fn send(&self, session_id: &str, cmd: AcpCommand) -> anyhow::Result<()> {
        // Do NOT hold the global table lock across the await points below:
        // `cmd_tx.send().await` blocks when the per-session command channel
        // (capacity 64) is full, and `pending_responders.lock().await` can
        // contend with the driver's hook callback. Holding `inner` across
        // either would stall every other session's send (sebas-9pz §2.5).
        //
        // Strategy: clone the one Arc we need under the lock, drop the lock,
        // then await. Clones are cheap (Arc bumps) and the map is only read
        // for lookup.
        match &cmd {
            AcpCommand::PermissionReply {
                request_id,
                decision,
                ..
            } => {
                let pending = {
                    let g = self.inner.lock().await;
                    let m = g
                        .get(session_id)
                        .ok_or_else(|| anyhow::anyhow!("unknown session"))?;
                    m.handle.pending_responders.clone()
                };
                let mut map = pending.lock().await;
                let slot = map.remove(request_id);
                if slot.is_none() {
                    let known: Vec<String> = map.keys().cloned().collect();
                    drop(map);
                    tracing::warn!(
                        request_id,
                        known_keys = ?known,
                        "no pending responder; dropping permission reply"
                    );
                    return Ok(());
                }
                let slot = slot.unwrap();
                drop(map);
                slot.send(decision.clone())
                    .map_err(|_| anyhow::anyhow!("permission responder dropped"))?;
                Ok(())
            }
            other => {
                let cmd_tx = {
                    let g = self.inner.lock().await;
                    let m = g
                        .get(session_id)
                        .ok_or_else(|| anyhow::anyhow!("unknown session"))?;
                    m.handle.cmd_tx.clone()
                };
                cmd_tx
                    .send(other.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("send to session cmd channel: {e}"))
            }
        }
    }

    pub async fn next_event(&self, session_id: &str) -> Option<AcpEvent> {
        // Same lock-scope fix as `send`: `rx.recv().await` can block for the
        // whole lifetime of a turn, so it must NOT run under the global
        // table lock (sebas-9pz §2.5).
        let rx = {
            let g = self.inner.lock().await;
            g.get(session_id)?.handle.evt_rx.clone()
        };
        let mut rx = rx.lock().await;
        rx.recv().await
    }

    pub async fn event_rx(&self, session_id: &str) -> Option<Arc<Mutex<mpsc::Receiver<AcpEvent>>>> {
        self.inner
            .lock()
            .await
            .get(session_id)
            .map(|m| m.handle.evt_rx.clone())
    }

    /// Stop every live session. Called on daemon shutdown so child
    /// processes are signalled before the state snapshot is written.
    pub async fn kill_all(&self) {
        let metas: Vec<SessionMeta> = self.inner.lock().await.drain().map(|(_, m)| m).collect();
        for m in metas {
            m.expected_exit
                .store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(tx) = m.handle.cancel_tx {
                let _ = tx.send(());
            }
        }
    }
}
