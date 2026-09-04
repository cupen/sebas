//! `SessionManager` — public API for spawning per-session third-party agents.
//!
//! Driver-agnostic: the manager holds a registry of [`crate::AgentDriver`]s
//! keyed by an open kind slug, and routes each session to the driver bound to
//! its kind. The dedicated Claude driver lives in `crate::claude::driver`; the
//! generic ACP driver in `crate::acp_driver`. The public surface
//! (`SessionManager`, `SessionStart`, `SpawnOutcome`, `AcpCommand`/`AcpEvent`)
//! keeps its semantics — only the spawn entry points gain a `kind` argument.

use crate::agent_driver::{AgentDriver, DriverConfig};
use crate::session::{AcpCommand, AcpEvent, AcpModelInfo, AcpSessionHandle, SessionMeta};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};

/// How the background session task should establish the agent session.
#[derive(Debug, Clone)]
pub enum SessionStart {
    /// Fresh conversation (a uuid is minted and passed to the driver).
    New,
    /// `resume`/load the given (previously persisted) conversation id.
    /// `routing_id` is the id sebas routes by; `acp_session_id` is the
    /// agent's real ACP session id when it differs from the routing id
    /// (native-ACP agents, e.g. opencode) — `Some` makes the driver load by
    /// that id, `None` keeps the legacy "load by routing id" behavior.
    Load {
        routing_id: String,
        acp_session_id: Option<String>,
    },
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
    /// The agent's real ACP session id when it differs from the routing id
    /// (native-ACP agents: `session/new` id on fresh spawn, the loaded
    /// conversation id on resume). `None` for Claude and for drivers that
    /// reported none. Persisted alongside the routing id so a later resume
    /// can load the conversation by the id the agent actually knows.
    pub acp_session_id: Option<String>,
    /// （add-acp-model-selection）会话建立时 agent 经 `configOptions` 暴露的
    /// 模型选择面（当前模型 + 可选列表）。`None` = agent 未暴露模型选项
    /// （webui 不显示模型下拉、不报错）。webui 把它存进 session 记录，
    /// 快照 API 暴露 `current_model` / `available_models`。
    pub model: Option<AcpModelInfo>,
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
        self.resume_session("claude", command, work_dir, extra_env, old_session_id, None)
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
        let outcome = self
            .spawn(kind, command, work_dir, extra_env, SessionStart::New)
            .await?;
        Ok(outcome.session_id)
    }

    /// Lazily respawn a previously persisted session: resume/load the old
    /// conversation id. `acp_session_id` is the agent's real ACP session id
    /// when it differs from the routing id (native-ACP agents, e.g.
    /// opencode) — `Some` makes the driver issue `session/load` with that id,
    /// `None` keeps the legacy routing-id-as-load-target behavior. Graceful
    /// fallback (sebas-dk8.4): if the driver rejects the resume (conversation
    /// files gone), a fresh session starts transparently with a NEW id and
    /// `SpawnOutcome.resumed == false`.
    pub async fn resume_session(
        &self,
        kind: &str,
        command: Vec<String>,
        work_dir: Option<String>,
        extra_env: Vec<(String, String)>,
        old_session_id: &str,
        acp_session_id: Option<String>,
    ) -> anyhow::Result<SpawnOutcome> {
        self.spawn(
            kind,
            command,
            work_dir,
            extra_env,
            SessionStart::Load {
                routing_id: old_session_id.to_string(),
                acp_session_id,
            },
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

        let (session_id, resume, load_session_id) = match &start {
            SessionStart::New => (uuid::Uuid::new_v4().to_string(), false, None),
            SessionStart::Load {
                routing_id,
                acp_session_id,
            } => (routing_id.clone(), true, acp_session_id.clone()),
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
            load_session_id,
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

        // The run loop must be polled for the handshake to fire: it performs
        // the ACP initialize + load/new inside the connect closure. Spawn it
        // BEFORE awaiting the handshake, otherwise it never runs and we
        // deadlock into the startup timeout.
        //
        // The wrapper's cleanup uses a shared "final routing id" slot: ACP
        // drivers may swap in a fresh id during the handshake, so the manager
        // publishes the resolved id after awaiting; Claude drivers (no
        // handshake) leave the slot empty and the wrapper falls back to
        // `handle.session_id`.
        let final_sid: Arc<std::sync::Mutex<Option<String>>> =
            Arc::new(std::sync::Mutex::new(None));
        let wrapper_final_sid = final_sid.clone();
        let wrapper_evt_tx = evt_tx.clone();
        let wrapper_inner = self.inner.clone();
        let wrapper_expected_exit = expected_exit.clone();
        let wrapper_terminal_sent = terminal_sent.clone();
        // Claude drivers (no handshake) never fill the slot; their routing id
        // is known up front and used as the wrapper's cleanup fallback.
        let wrapper_fallback_sid = handle.session_id.clone();
        let run_task = tokio::spawn(async move {
            handle.run.await;
            // Terminal-event guarantee (openspec/specs/acp-driver/spec.md): a
            // session that dies without an explicit kill surfaces exactly one
            // Error{terminal:true}; then the table entry is dropped so send()
            // fails fast and all senders close the stream. Eager removal is
            // safe: consumers clone `Arc<Mutex<Receiver>>` before the session
            // can die, so buffered terminal events survive map removal.
            let sid = wrapper_final_sid
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(wrapper_fallback_sid);
            if !wrapper_expected_exit.load(std::sync::atomic::Ordering::SeqCst)
                && !wrapper_terminal_sent.load(std::sync::atomic::Ordering::SeqCst)
            {
                let _ = wrapper_evt_tx
                    .send(AcpEvent::Error {
                        session_id: sid.clone(),
                        message: "agent process exited".into(),
                        terminal: true,
                    })
                    .await;
            }
            wrapper_inner.lock().await.remove(&sid);
        });

        // Await the definitive routing id + `resumed` (+ optional real ACP
        // session id + model info) under the startup timeout before inserting
        // the session. ACP drivers publish via the handshake; Claude drivers
        // return `None` and are already final.
        let (sid, resumed, acp_session_id, model) = match handle.handshake {
            Some(rx) => match tokio::time::timeout(entry.startup_timeout, rx).await {
                Ok(Ok(pair)) => pair,
                Ok(Err(_)) => {
                    run_task.abort();
                    return Err(anyhow::anyhow!(
                        "agent driver handshake channel closed before completion"
                    ));
                }
                Err(_) => {
                    run_task.abort();
                    return Err(anyhow::anyhow!(
                        "agent driver handshake timed out after {:?}",
                        entry.startup_timeout
                    ));
                }
            },
            None => (
                handle.session_id.clone(),
                handle.resumed,
                handle.acp_session_id,
                handle.model,
            ),
        };
        // Publish the resolved id to the wrapper so its cleanup targets the
        // right table entry (ACP fresh fallback mints a new one).
        *final_sid.lock().unwrap() = Some(sid.clone());

        let acp_handle = AcpSessionHandle {
            session_id: sid.clone(),
            cmd_tx,
            evt_rx: Arc::new(Mutex::new(evt_rx)),
            cancel_tx: Some(cancel_tx),
            pending_responders,
        };
        self.inner.lock().await.insert(
            sid.clone(),
            SessionMeta {
                session_id: sid.clone(),
                acp_session_id: acp_session_id.clone(),
                model: model.clone(),
                handle: acp_handle,
                expected_exit,
            },
        );

        Ok(SpawnOutcome {
            session_id: sid,
            resumed,
            acp_session_id,
            model,
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

    /// The agent's real ACP session id recorded for `session_id` (the
    /// `session/new` id on a fresh spawn, the loaded conversation id on a
    /// successful resume), when the driver reported one and the session is
    /// still live in the manager's table. This is the id a future resume
    /// must hand to `session/load`. `None` when the session is unknown,
    /// the driver reported none (e.g. Claude), or the session's lifetime
    /// ended (the table entry is dropped on termination).
    pub async fn get_acp_session_id(&self, session_id: &str) -> Option<String> {
        self.inner.lock().await.get(session_id)?.acp_session_id.clone()
    }

    /// The model selection surface reported by the driver at session
    /// establishment（`configOptions` 里的 model 类选项），当会话仍在表中。
    /// `None` = 会话未知、driver 未上报（如 Claude），或会话已终止。
    pub async fn get_model_info(&self, session_id: &str) -> Option<AcpModelInfo> {
        self.inner.lock().await.get(session_id)?.model.clone()
    }

    /// Issue `AcpCommand::SetModel` for a live session（走命令通道，driver
    /// 发标准 `session/set_config_option`）。失败（无效模型 / agent 无此
    /// 能力 / 会话已终止）以显式错误返回，调用方应呈现给用户。
    pub async fn set_model(
        &self,
        session_id: &str,
        model_id: impl Into<String>,
    ) -> anyhow::Result<()> {
        let cmd = AcpCommand::SetModel {
            session_id: session_id.to_string(),
            model_id: model_id.into(),
        };
        self.send(session_id, cmd).await
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
