//! `SessionManager` — public API for spawning per-session third-party agents.
//!
//! Driver-agnostic: the manager holds a registry of [`crate::AgentDriver`]s
//! keyed by an open kind slug, and routes each session to the driver bound to
//! its kind. The dedicated Claude driver lives in `crate::claude::driver`; the
//! generic ACP driver in `crate::acp_driver`. The public surface
//! (`SessionManager`, `SessionStart`, `SpawnOutcome`, `AcpCommand`/`AcpEvent`)
//! keeps its semantics — only the spawn entry points gain a `kind` argument.

use crate::agent_driver::{AgentDriver, DriverConfig};
use crate::session::{AcpCommand, AcpEvent, AcpSessionHandle, SessionMeta};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};

/// How the background session task should establish the agent session.
#[derive(Debug, Clone)]
pub enum SessionStart {
    /// Fresh conversation (a uuid is minted and passed to the driver).
    New,
    /// `resume`/load the given (previously persisted) conversation id.
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

/// A registered agent: its driver plus the per-kind startup timeout.
#[derive(Clone)]
pub struct AgentEntry {
    pub driver: Arc<dyn AgentDriver>,
    pub startup_timeout: std::time::Duration,
}

pub struct SessionManager {
    inner: Arc<Mutex<HashMap<String, SessionMeta>>>,
    agents: HashMap<String, AgentEntry>,
    default: String,
}

impl SessionManager {
    /// Build a manager from a kind → driver registry plus the default kind.
    pub fn new(default: String, agents: HashMap<String, AgentEntry>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            agents,
            default,
        }
    }

    /// Convenience: a single-Claude registry with the given startup timeout.
    /// Used by the integration tests and simple single-agent deployments.
    pub fn claude_only(startup_timeout: std::time::Duration) -> Self {
        let mut agents = HashMap::new();
        agents.insert(
            "claude".to_string(),
            AgentEntry {
                driver: Arc::new(super::driver::ClaudeDriver),
                startup_timeout,
            },
        );
        Self::new("claude".to_string(), agents)
    }

    /// Convenience: spawn the dedicated Claude driver with `path` as the
    /// binary and `args` appended. Delegates to the kind-keyed
    /// [`SessionManager::create_session`] under the `"claude"` kind.
    pub async fn create_claude_session(
        &self,
        path: &str,
        args: Vec<String>,
        work_dir: Option<String>,
        extra_env: Vec<(String, String)>,
        prompt: String,
    ) -> anyhow::Result<String> {
        let mut command = vec![path.to_string()];
        command.extend(args);
        self.create_session("claude", command, work_dir, extra_env, prompt)
            .await
    }

    /// Convenience: resume the dedicated Claude driver with `path`/`args`,
    /// delegating to [`SessionManager::resume_session`] under `"claude"`.
    pub async fn resume_claude_session(
        &self,
        path: &str,
        args: Vec<String>,
        work_dir: Option<String>,
        extra_env: Vec<(String, String)>,
        old_session_id: &str,
    ) -> anyhow::Result<SpawnOutcome> {
        let mut command = vec![path.to_string()];
        command.extend(args);
        self.resume_session("claude", command, work_dir, extra_env, old_session_id)
            .await
    }

    /// Resolve a kind slug (empty means the configured default).
    fn resolve_kind(&self, kind: &str) -> anyhow::Result<String> {
        let k = if kind.is_empty() {
            self.default.clone()
        } else {
            kind.to_string()
        };
        if self.agents.contains_key(&k) {
            Ok(k)
        } else {
            anyhow::bail!("unknown agent kind {k:?}")
        }
    }

    fn entry(&self, kind: &str) -> anyhow::Result<&AgentEntry> {
        self.agents
            .get(kind)
            .ok_or_else(|| anyhow::anyhow!("unknown agent kind {kind:?}"))
    }

    /// `prompt` is part of the public API surface but is **not** sent here —
    /// the prompt is forwarded exactly once via `AcpCommand::CreateSession`
    /// (or `ContinueSession`) through `send`. (Unchanged contract.)
    ///
    /// `command` is the full argv (executable + args, including any
    /// provider-derived extra args computed by the caller). `extra_env` is
    /// merged into the child's environment on top of the OS env (provider
    /// keys like `ANTHROPIC_BASE_URL`).
    pub async fn create_session(
        &self,
        kind: &str,
        command: Vec<String>,
        work_dir: Option<String>,
        extra_env: Vec<(String, String)>,
        _prompt: String,
    ) -> anyhow::Result<String> {
        Ok(self
            .spawn(kind, command, work_dir, extra_env, SessionStart::New)
            .await?
            .session_id)
    }

    /// Lazily respawn a previously persisted session: resume/load the old
    /// conversation id. Graceful fallback (sebas-dk8.4): if the driver rejects
    /// the resume (conversation files gone), a fresh session starts
    /// transparently with a NEW id and `SpawnOutcome.resumed == false`.
    pub async fn resume_session(
        &self,
        kind: &str,
        command: Vec<String>,
        work_dir: Option<String>,
        extra_env: Vec<(String, String)>,
        old_session_id: &str,
    ) -> anyhow::Result<SpawnOutcome> {
        self.spawn(
            kind,
            command,
            work_dir,
            extra_env,
            SessionStart::Load(old_session_id.to_string()),
        )
        .await
    }

    /// Spawn a per-session agent subprocess via the driver bound to `kind`,
    /// and start the driver's read/command loop. Returns the id callers use
    /// as the routing key for `send`/`event_rx`/`kill`.
    async fn spawn(
        &self,
        kind: &str,
        command: Vec<String>,
        work_dir: Option<String>,
        extra_env: Vec<(String, String)>,
        start: SessionStart,
    ) -> anyhow::Result<SpawnOutcome> {
        let kind = self.resolve_kind(kind)?;
        let entry = self.entry(&kind)?;

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

        let cfg = DriverConfig {
            kind_slug: kind.clone(),
            command,
            work_dir,
            extra_env,
            session_id: session_id.clone(),
            resume,
            startup_timeout: entry.startup_timeout,
            evt_tx: evt_tx.clone(),
            cmd_rx,
            cancel_rx,
            pending_perms: pending_responders.clone(),
            terminal_sent: terminal_sent.clone(),
        };

        // The driver establishes the session and boxes its run loop. Any
        // resume-rejection fallback is the driver's own concern (claude-only).
        let handle = entry.driver.spawn(cfg).await?;

        tokio::spawn({
            let inner = self.inner.clone();
            let sid = handle.session_id.clone();
            let expected_exit = expected_exit.clone();
            let run = handle.run;
            async move {
                run.await;
                // Terminal-event guarantee (openspec/specs/acp-driver/spec.md): a session that dies
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
                inner.lock().await.remove(&sid);
            }
        });

        let acp_handle = AcpSessionHandle {
            session_id: handle.session_id.clone(),
            cmd_tx,
            evt_rx: Arc::new(Mutex::new(evt_rx)),
            cancel_tx: Some(cancel_tx),
            pending_responders,
        };
        self.inner.lock().await.insert(
            handle.session_id.clone(),
            SessionMeta {
                session_id: handle.session_id.clone(),
                handle: acp_handle,
                expected_exit,
            },
        );

        Ok(SpawnOutcome {
            session_id: handle.session_id,
            resumed: handle.resumed,
        })
    }

    /// Stop one session. Drops the cancel sender; the driver loop exits and
    /// the child is killed.
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
    ///   permission hook. Does not touch the command channel.
    /// - `Cancel` cancels the current turn; the driver heals the session.
    /// - Everything else (CreateSession, ContinueSession) is forwarded to
    ///   the command channel for the driver to handle.
    pub async fn send(&self, session_id: &str, cmd: AcpCommand) -> anyhow::Result<()> {
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
