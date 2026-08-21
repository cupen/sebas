//! Engine adapter: drives a real `claude` CLI via `cc-agent-sdk`
//! (stream-json + control protocol) and surfaces the unchanged
//! `AcpEvent`/`AcpCommand` vocabulary the router consumes.
//!
//! Post-ACP design (docs/superpowers/specs/2026-08-06-claude-direct-sdk-refactor-design.md):
//! - One `ClaudeClient` per sebas session; the sebas routing id IS the claude
//!   conversation id. Fresh spawns mint a uuid and pass `--session-id`;
//!   resume passes ONLY `--resume <id>` — the real CLI rejects
//!   `--session-id` combined with `--resume`/`--continue` unless
//!   `--fork-session` is also given (and forking would change the id).
//! - Permissions ride the PreToolUse hook callback (process-internal,
//!   control-request correlated — no socket/hook-script/positional pairing).
//! - `/cancel` = `interrupt()` + respawn-with-resume: the CLI is unusable
//!   after an interrupt (spike §S6), so we transparently reconnect with
//!   `resume` to keep D4 semantics ("cancel the turn, keep the session").
//! - `setting_sources = Some(vec![])` hermetically isolates the child from
//!   the host user's settings/hooks (spike §8b).

use crate::session::{AcpCommand, AcpEvent, Decision, ResponderSlot, TurnUsage};
use claude_agent_sdk::{
    ClaudeAgentOptions, ClaudeClient, ContentBlock, HookCallback, HookEvent, HookInput,
    HookJsonOutput, HookMatcher, HookSpecificOutput, Message, PreToolUseHookSpecificOutput,
    SyncHookJsonOutput,
};
use futures::{FutureExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, oneshot};

/// Everything needed to establish one claude-backed session.
pub struct ConnectConfig {
    pub claude_path: String,
    pub claude_args: Vec<String>,
    pub work_dir: Option<String>,
    /// Additional env vars merged into the child process's environment on
    /// top of the OS-given env. Used by sebas to inject provider-driven
    /// keys (`ANTHROPIC_BASE_URL`, `OPENAI_API_KEY`, etc.) at spawn time
    /// (bead sebas-63f.8). Empty when no override applies (Off mode).
    pub extra_env: Vec<(String, String)>,
    /// The sebas routing id (uuid minted by the manager; also becomes the
    /// claude conversation id via `--session-id`).
    pub session_id: String,
    /// True → `options.resume = session_id` (lazy respawn / post-cancel heal).
    pub resume: bool,
    pub startup_timeout: Duration,
    pub evt_tx: mpsc::Sender<AcpEvent>,
    pub pending_perms: Arc<Mutex<HashMap<String, ResponderSlot>>>,
    /// Set when the driver itself emits a terminal Error, so the manager's
    /// wrapper doesn't synthesize a second one ("agent process exited").
    pub terminal_sent: Arc<std::sync::atomic::AtomicBool>,
}

pub struct CcDriver {
    client: ClaudeClient,
    session_id: String,
    cfg: DriverCfg,
    extra_env: Vec<(String, String)>,
    evt_tx: mpsc::Sender<AcpEvent>,
    pending_perms: Arc<Mutex<HashMap<String, ResponderSlot>>>,
    /// tool_use_id → tool_name, so User(tool_result) frames can emit ToolEnd
    /// with the tool name (the frames themselves only carry the id).
    tool_names: HashMap<String, String>,
    terminal_sent: Arc<std::sync::atomic::AtomicBool>,
    /// Capped tail of the child's stderr, appended to terminal errors so a
    /// crash usually carries its own explanation (the SDK pipes stderr but
    /// drops it unless a callback is installed).
    stderr_tail: Arc<std::sync::Mutex<String>>,
    /// Hang detection (sebas-9pz ①): `Instant` of the last activity
    /// (any `Ok` message on the stream, or a permission request hand-off).
    /// When the child produces nothing for `HANG_TIMEOUT`, the driver
    /// escalates: interrupt() ×3 → disconnect (≈SIGTERM) → 5s → drop
    /// (≈SIGKILL). Tied to spec §4.1 "5min 无任何 notification".
    last_activity: tokio::time::Instant,
    /// Escalation stage: 0..=3 interrupts already sent for the current hang.
    hang_stage: u8,
    /// True while a PreToolUse permission prompt is parked awaiting the
    /// user's click (spec §4.1: permission wait is "永不超时"). Hang
    /// detection is suspended while this is set, otherwise a slow user
    /// click would look exactly like a hung child.
    waiting_permission: Arc<std::sync::atomic::AtomicBool>,
    /// True while a turn is in progress (between CreateSession/ContinueSession
    /// and the matching Message::Result). Hang detection only fires when
    /// a turn is active — otherwise the child is idle (waiting for the next
    /// prompt) and must not be killed.
    turn_active: bool,
}

/// Why a connect attempt failed. `ResumeRejected` is carved out so the
/// manager can transparently fall back to a fresh session (sebas-dk8.4)
/// instead of surfacing a raw spawn error for a very expected case
/// (daemon restart after claude's session files were cleaned).
#[derive(Debug)]
pub enum ConnectError {
    /// claude rejected `resume` — the conversation id is unknown to it
    /// (its stderr said "No conversation found").
    ResumeRejected,
    Other(anyhow::Error),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ResumeRejected => write!(f, "claude rejected resume: conversation not found"),
            Self::Other(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for ConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ResumeRejected => None,
            Self::Other(e) => Some(e.as_ref()),
        }
    }
}

/// The parts of ConnectConfig needed again for a respawn (post-cancel).
struct DriverCfg {
    claude_path: String,
    claude_args: Vec<String>,
    work_dir: Option<String>,
    startup_timeout: Duration,
}

impl CcDriver {
    /// Spawn the claude child and complete the SDK initialize handshake.
    /// On timeout the client is dropped, which SIGKILLs the child
    /// (`SubprocessTransport::drop` → `start_kill`).
    ///
    /// Resume rejection (sebas-dk8.4): the SDK awaits the initialize
    /// control-response forever even when the child already exited (pending
    /// control oneshots are never errored on stdout EOF), so a rejected
    /// `resume` would otherwise hang until `startup_timeout`. We race the
    /// handshake against the child's stderr: the moment claude prints
    /// "No conversation found" we return `ConnectError::ResumeRejected` —
    /// fast and exact (fresh spawns never print that line).
    pub async fn connect(cfg: ConnectConfig) -> Result<Self, ConnectError> {
        let ConnectConfig {
            claude_path,
            claude_args,
            work_dir,
            extra_env,
            session_id,
            resume,
            startup_timeout,
            evt_tx,
            pending_perms,
            terminal_sent,
        } = cfg;

        let mut extra_args = args_to_extra_args(&claude_args);
        // Only fresh spawns may pin the conversation id: the real CLI
        // rejects `--session-id` together with `--resume`/`--continue`
        // unless `--fork-session` is also specified (and forking would
        // change the id we route by). On resume the conversation keeps its
        // existing id, which IS our `session_id`.
        if !resume {
            extra_args.insert("session-id".into(), Some(session_id.clone()));
        }

        // Provider-driven env (sebas-63f.8): injected into the child so
        // claude hits the resolved upstream URL/token rather than the OS env.
        let env_map: std::collections::HashMap<String, String> =
            extra_env.iter().cloned().collect();

        let waiting_permission = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cb = permission_hook(
            session_id.clone(),
            evt_tx.clone(),
            pending_perms.clone(),
            waiting_permission.clone(),
        );
        let mut hooks: HashMap<HookEvent, Vec<HookMatcher>> = HashMap::new();
        hooks.insert(
            HookEvent::PreToolUse,
            vec![HookMatcher::builder().hooks(vec![cb]).build()],
        );

        // Capture child stderr (capped) for diagnostics + resume rejection.
        let stderr_tail: Arc<std::sync::Mutex<String>> =
            Arc::new(std::sync::Mutex::new(String::new()));
        let resume_rejected = Arc::new(tokio::sync::Notify::new());
        let stderr_cb = {
            let tail = stderr_tail.clone();
            let rejected = resume_rejected.clone();
            Arc::new(move |line: String| {
                if line.contains("No conversation found") {
                    rejected.notify_one();
                }
                tracing::debug!(stderr = %line.trim_end(), "claude child");
                let mut b = tail.lock().unwrap_or_else(|p| p.into_inner());
                const CAP: usize = 4096;
                if b.len() + line.len() > CAP {
                    // Drop the oldest bytes, landing on a char boundary; a
                    // single oversized line simply empties the buffer first.
                    let mut from = b.len().saturating_sub(CAP.saturating_sub(line.len()));
                    while from < b.len() && !b.is_char_boundary(from) {
                        from += 1;
                    }
                    b.drain(..from);
                }
                b.push_str(&line);
            }) as Arc<dyn Fn(String) + Send + Sync>
        };

        let options = ClaudeAgentOptions {
            cli_path: Some(claude_path.clone().into()),
            cwd: work_dir.clone().map(Into::into),
            hooks: Some(hooks),
            env: env_map,
            extra_args,
            resume: if resume {
                Some(session_id.clone())
            } else {
                None
            },
            // Hermetic: never load the host user's settings/hooks (spike §8b).
            setting_sources: Some(vec![]),
            stderr_callback: Some(stderr_cb),
            ..Default::default()
        };

        let mut client = ClaudeClient::new(options);
        let res = {
            // Scoped: the pinned connect future borrows &mut client; it
            // must drop at block end so `client` can move into Self below.
            let connect = client.connect();
            tokio::pin!(connect);
            tokio::select! {
                r = tokio::time::timeout(startup_timeout, &mut connect) => r,
                // Armed only for resume attempts (fresh spawns never print
                // the line). `Notify` holds one permit, so a line printed
                // before we get here still resolves immediately.
                _ = resume_rejected.notified(), if resume => {
                    return Err(ConnectError::ResumeRejected);
                }
            }
        };
        match res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Err(ConnectError::Other(anyhow::anyhow!(
                    "{e:#}{}",
                    stderr_suffix(&stderr_tail)
                )));
            }
            Err(_) => {
                return Err(ConnectError::Other(anyhow::anyhow!(
                    "acp session start timed out after {:?}{}",
                    startup_timeout,
                    stderr_suffix(&stderr_tail)
                )));
            }
        }

        Ok(Self {
            client,
            session_id,
            cfg: DriverCfg {
                claude_path,
                claude_args,
                work_dir,
                startup_timeout,
            },
            extra_env,
            evt_tx,
            pending_perms,
            tool_names: HashMap::new(),
            terminal_sent,
            stderr_tail,
            last_activity: tokio::time::Instant::now(),
            hang_stage: 0,
            waiting_permission,
            turn_active: false,
        })
    }

    /// The session read/command loop. Exits when the cancel oneshot fires
    /// (kill), the command channel closes (manager dropped the handle), a
    /// terminal error is emitted, or the watchdog declares the child dead.
    ///
    /// Watchdog (sebas-9pz substance): the SDK's reader ends silently on
    /// child stdout EOF — `receive_response` then pends forever (the channel
    /// never closes while the client lives), so a crashed/dead CLI is
    /// otherwise invisible. Once a second we send a harmless control request
    /// (`set_permission_mode(default)`, answered instantly by any live CLI);
    /// a transport error or 1.5s timeout means the child is gone or hung →
    /// terminal Error.
    pub async fn run(
        mut self,
        mut cmd_rx: mpsc::Receiver<AcpCommand>,
        mut cancel_rx: oneshot::Receiver<()>,
    ) {
        // Transient per-iteration select tag; not on a hot path, so the
        // large Message payload inside Msg is fine.
        #[allow(clippy::large_enum_variant)]
        enum Sel {
            Cmd(Option<AcpCommand>),
            Kill,
            Msg(Option<Result<Message, claude_agent_sdk::ClaudeError>>),
            Tick,
        }
        let mut watchdog = tokio::time::interval(Duration::from_secs(1));
        watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            // The stream borrows &self.client; it is dropped at the end of
            // this block so commands needing &mut self.client can run.
            // Buffered messages survive recreation (the SDK shares one
            // channel receiver behind an Arc).
            let sel = {
                let stream = self.client.receive_response();
                tokio::pin!(stream);
                tokio::select! {
                    biased;
                    _ = &mut cancel_rx => Sel::Kill,
                    cmd = cmd_rx.recv() => Sel::Cmd(cmd),
                    msg = stream.next() => Sel::Msg(msg),
                    _ = watchdog.tick() => Sel::Tick,
                }
            };
            match sel {
                Sel::Kill | Sel::Cmd(None) => break,
                Sel::Tick => {
                    // `set_permission_mode(default)` is a harmless no-op the
                    // CLI always answers; any failure/timeout ⇒ child gone.
                    let probe = self
                        .client
                        .set_permission_mode(claude_agent_sdk::PermissionMode::Default);
                    let dead = match tokio::time::timeout(Duration::from_millis(1500), probe).await
                    {
                        Ok(Ok(())) => false,
                        Ok(Err(_)) | Err(_) => true,
                    };
                    if dead {
                        self.terminal("agent process exited or hung (watchdog)")
                            .await;
                        return;
                    }

                    // Hang detection (sebas-9pz ①, spec §4.1): the child is
                    // alive but silent for HANG_TIMEOUT. The SDK exposes no
                    // process handle, so SIGTERM/SIGKILL are approximated with
                    // the SDK's own escalation: interrupt() (cancel, ×3), then
                    // disconnect() (closes stdin ≈ SIGTERM), then a 5s grace
                    // before the driver returns — dropping `self.client`
                    // SIGKILLs the child (SubprocessTransport::drop).
                    // HANG_TIMEOUT defaults to spec §4.1's 5min; tests override
                    // via SEBAS_HANG_TIMEOUT_SECS so a hang regression test
                    // doesn't have to sleep 5 minutes.
                    let hang_timeout = std::env::var("SEBAS_HANG_TIMEOUT_SECS")
                        .ok()
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(Duration::from_secs)
                        .unwrap_or(Duration::from_secs(5 * 60));
                    const ESCALATE_GRACE: Duration = Duration::from_secs(2);
                    const SIGKILL_GRACE: Duration = Duration::from_secs(5);
                    const MAX_INTERRUPTS: u8 = 3;
                    // Permission wait suspends hang detection (spec §4.1:
                    // "永不超时"). A slow user click must not look like a
                    // hung child.
                    let awaiting_user = self
                        .waiting_permission
                        .load(std::sync::atomic::Ordering::SeqCst);
                    if awaiting_user {
                        continue;
                    }
                    if self.last_activity.elapsed() > hang_timeout
                        && self.turn_active
                        && self.hang_stage < 3
                    {
                        self.hang_stage += 1;
                        tracing::warn!(
                            session_id = %self.session_id,
                            stage = self.hang_stage,
                            "agent silent for 5m; escalating (interrupt {}/{MAX_INTERRUPTS})",
                            self.hang_stage
                        );
                        // `interrupt()` kills the current turn; on a live-but-
                        // hung child it either wakes it (activity resumes, the
                        // next Msg resets last_activity) or errors (child gone
                        // → next probe trips `dead`).
                        let _ = self.client.interrupt().await;
                        tokio::time::sleep(ESCALATE_GRACE).await;
                        continue;
                    }
                    if self.hang_stage >= MAX_INTERRUPTS {
                        tracing::error!(
                            session_id = %self.session_id,
                            "agent unresponsive after 3 interrupts; force-restarting (SIGTERM→SIGKILL)"
                        );
                        // ≈SIGTERM: close the child's stdin and await exit.
                        let _ = self.client.disconnect().await;
                        tokio::time::sleep(SIGKILL_GRACE).await;
                        self.terminal("agent hung (no activity for 5m; 3 cancels failed)")
                            .await;
                        // Returning drops `self.client` → SubprocessTransport
                        // Drop → start_kill (≈SIGKILL) for any straggler.
                        return;
                    }
                }
                Sel::Cmd(Some(AcpCommand::Cancel { .. })) => {
                    if !self.handle_cancel().await {
                        return;
                    }
                }
                Sel::Cmd(Some(AcpCommand::CreateSession { prompt, .. }))
                | Sel::Cmd(Some(AcpCommand::ContinueSession { prompt, .. })) => {
                    self.turn_active = true;
                    if let Err(e) = self.client.query(prompt).await {
                        self.terminal(&format!("session/prompt failed: {e}")).await;
                        return;
                    }
                }
                Sel::Cmd(Some(AcpCommand::PermissionReply { .. })) => {
                    // Replies travel via the pending map (manager.send
                    // intercepts before the channel); never expected here.
                    tracing::debug!("ignoring unexpected PermissionReply on session channel");
                }
                Sel::Msg(Some(Ok(m))) => {
                    // Any real message from the child counts as activity:
                    // resets the hang timer and clears the escalation stage
                    // (sebas-9pz ①).
                    self.last_activity = tokio::time::Instant::now();
                    self.hang_stage = 0;
                    // Message::Result = turn finished (child will go silent
                    // until next prompt). Clear turn_active so hang detection
                    // doesn't mis-kill an idle-but-healthy child.
                    if matches!(&m, Message::Result(_)) {
                        self.turn_active = false;
                    }
                    for evt in map_message(&self.session_id, &mut self.tool_names, &m) {
                        let is_terminal = matches!(evt, AcpEvent::Error { terminal: true, .. });
                        if self.evt_tx.send(evt).await.is_err() || is_terminal {
                            return;
                        }
                    }
                }
                Sel::Msg(Some(Err(e))) => {
                    // MessageParseError = unknown message type from CLI, not
                    // a real error. Log the raw data and continue instead of
                    // killing the session.
                    if let claude_agent_sdk::ClaudeError::MessageParse(inner) = &e {
                        if let Some(raw) = &inner.data {
                            tracing::warn!(
                                raw = %serde_json::to_string(raw).unwrap_or_default(),
                                "ignoring unknown message type from claude"
                            );
                        } else {
                            tracing::warn!("ignoring unknown message from claude");
                        }
                        continue;
                    }
                    self.terminal(&format!("claude stream error: {e}")).await;
                    return;
                }
                Sel::Msg(None) => return, // child stdout EOF — process exited
            }
        }
        let _ = self.client.disconnect().await;
    }

    /// D4 under the new engine: `interrupt()` kills the turn but leaves the
    /// CLI unusable (spike §S6), so after the error result lands we
    /// disconnect and transparently respawn with `resume` — the conversation
    /// survives and the next prompt works. Emits `Finished` (turn aborted
    /// cleanly) on success, terminal `Error` if the heal fails.
    /// Returns false when the loop must exit.
    async fn handle_cancel(&mut self) -> bool {
        if let Err(e) = self.client.interrupt().await {
            self.terminal(&format!("interrupt failed: {e}")).await;
            return false;
        }
        // Drain until the post-interrupt result frame (bounded; the fake
        // exits right after it, real CLI likewise).
        let drain = async {
            let stream = self.client.receive_response();
            tokio::pin!(stream);
            while let Some(item) = stream.next().await {
                match item {
                    Ok(Message::Result(_)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(5), drain).await;
        let _ = self.client.disconnect().await;

        let cfg = ConnectConfig {
            claude_path: self.cfg.claude_path.clone(),
            claude_args: self.cfg.claude_args.clone(),
            work_dir: self.cfg.work_dir.clone(),
            extra_env: self.extra_env.clone(),
            session_id: self.session_id.clone(),
            resume: true,
            startup_timeout: self.cfg.startup_timeout,
            evt_tx: self.evt_tx.clone(),
            pending_perms: self.pending_perms.clone(),
            terminal_sent: self.terminal_sent.clone(),
        };
        match Self::connect(cfg).await {
            Ok(fresh) => {
                *self = fresh;
                // Turn aborted; card goes ✅ and the queue may drain.
                let _ = self
                    .evt_tx
                    .send(AcpEvent::Finished {
                        session_id: self.session_id.clone(),
                    })
                    .await;
                true
            }
            Err(e) => {
                self.terminal(&format!("cancel respawn failed: {e}")).await;
                false
            }
        }
    }

    async fn terminal(&self, message: &str) {
        self.terminal_sent
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = self
            .evt_tx
            .send(AcpEvent::Error {
                session_id: self.session_id.clone(),
                message: format!("{message}{}", stderr_suffix(&self.stderr_tail)),
                terminal: true,
            })
            .await;
    }
}

/// Render the captured child stderr as an error-message suffix ("" when
/// the child said nothing — e.g. a silent hang).
fn stderr_suffix(tail: &Arc<std::sync::Mutex<String>>) -> String {
    let b = tail.lock().unwrap_or_else(|p| p.into_inner());
    let t = b.trim();
    if t.is_empty() {
        String::new()
    } else {
        format!("; claude stderr: {t}")
    }
}

/// Build the PreToolUse hook callback that bridges a claude permission
/// prompt into `AcpEvent::PermissionRequest` and parks the decision oneshot.
/// The manager's `send(PermissionReply)` resolves the oneshot.
fn permission_hook(
    session_id: String,
    evt_tx: mpsc::Sender<AcpEvent>,
    pending: Arc<Mutex<HashMap<String, ResponderSlot>>>,
    waiting_permission: Arc<std::sync::atomic::AtomicBool>,
) -> HookCallback {
    use std::sync::atomic::Ordering;
    Arc::new(move |input: HookInput, tool_use_id: Option<String>, _ctx| {
        let session_id = session_id.clone();
        let evt_tx = evt_tx.clone();
        let pending = pending.clone();
        let waiting = waiting_permission.clone();
        async move {
            let HookInput::PreToolUse(pre) = input else {
                return allow_output("non-PreToolUse hook passthrough");
            };
            let request_id = tool_use_id.unwrap_or_else(|| format!("req-{}", uuid::Uuid::new_v4()));
            let (tx, rx) = oneshot::channel();
            pending.lock().await.insert(request_id.clone(), tx);
            // Suspend hang detection while the user decides (spec §4.1:
            // permission wait never times out). Cleared on decision (or when
            // the oneshot drops — the Err arm below).
            waiting.store(true, Ordering::SeqCst);
            // Fire-and-forget: if the router is gone, deny (fail closed).
            let _ = evt_tx
                .send(AcpEvent::PermissionRequest {
                    session_id,
                    request_id,
                    tool_name: pre.tool_name,
                    args: pre.tool_input,
                })
                .await;
            let out = match rx.await {
                Ok(Decision::AllowOnce) | Ok(Decision::AllowSession) => {
                    allow_output("allowed by sebas user")
                }
                Ok(Decision::Deny) | Err(_) => deny_output("denied by sebas user"),
            };
            waiting.store(false, Ordering::SeqCst);
            out
        }
        .boxed()
    })
}

fn allow_output(reason: &str) -> HookJsonOutput {
    HookJsonOutput::Sync(SyncHookJsonOutput {
        hook_specific_output: Some(HookSpecificOutput::PreToolUse(
            PreToolUseHookSpecificOutput {
                permission_decision: Some("allow".into()),
                permission_decision_reason: Some(reason.into()),
                updated_input: None,
            },
        )),
        ..Default::default()
    })
}

fn deny_output(reason: &str) -> HookJsonOutput {
    HookJsonOutput::Sync(SyncHookJsonOutput {
        hook_specific_output: Some(HookSpecificOutput::PreToolUse(
            PreToolUseHookSpecificOutput {
                permission_decision: Some("deny".into()),
                permission_decision_reason: Some(reason.into()),
                updated_input: None,
            },
        )),
        ..Default::default()
    })
}

/// Convert a loose `["--model", "x", "--verbose"]` argv list into the SDK's
/// `extra_args` map shape: flags without a following non-flag value become
/// bare keys. Non-flag bare tokens are dropped with a warning (the SDK's
/// map shape cannot express positionals).
fn args_to_extra_args(args: &[String]) -> HashMap<String, Option<String>> {
    let mut out = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(key) = a.strip_prefix("--") {
            let value = match args.get(i + 1) {
                Some(v) if !v.starts_with("--") => {
                    i += 1;
                    Some(v.clone())
                }
                _ => None,
            };
            out.insert(key.to_string(), value);
        } else {
            tracing::warn!(arg = %a, "dropping positional claude arg (extra_args cannot express it)");
        }
        i += 1;
    }
    out
}

/// Translate one SDK `Message` into zero or more `AcpEvent`s.
/// Pure except for `tool_names` bookkeeping (tool_use id → name so
/// User(tool_result) frames can name the tool in `ToolEnd`).
///
/// Mapping notes (spike §4.2):
/// - `Assistant` blocks arrive whole (partials disabled — parity with the
///   bridge's v2.1.220 envelope mode): text → TextDelta, thinking →
///   ThinkingDelta, tool_use → ToolStart.
/// - Tool results ride `Message::User` frames — walked as raw JSON so SDK
///   type strictness (e.g. missing optional fields) can't drop them.
/// - `Result{is_error:false}` → Finished; `is_error:true` → terminal Error
///   (post-error CLI state is unknown; the honest mapping is session death).
pub(crate) fn map_message(
    session_id: &str,
    tool_names: &mut HashMap<String, String>,
    msg: &Message,
) -> Vec<AcpEvent> {
    let sid = || session_id.to_string();
    match msg {
        Message::Assistant(a) => {
            let mut events: Vec<AcpEvent> = a
                .message
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text(t) if !t.text.is_empty() => Some(AcpEvent::TextDelta {
                        session_id: sid(),
                        delta: t.text.clone(),
                    }),
                    ContentBlock::Thinking(t) if !t.thinking.is_empty() => {
                        Some(AcpEvent::ThinkingDelta {
                            session_id: sid(),
                            delta: t.thinking.clone(),
                        })
                    }
                    ContentBlock::ToolUse(t) => {
                        tool_names.insert(t.id.clone(), t.name.clone());
                        Some(AcpEvent::ToolStart {
                            session_id: sid(),
                            tool_name: t.name.clone(),
                            args: t.input.clone(),
                        })
                    }
                    _ => None,
                })
                .collect();
            // Extract model name and token usage from the assistant message.
            if let Some(usage) = &a.message.usage {
                let input = usage.get("input_tokens").and_then(|v| v.as_u64());
                let output = usage.get("output_tokens").and_then(|v| v.as_u64());
                let cache_read = usage
                    .get("cache_read_input_tokens")
                    .and_then(|v| v.as_u64());
                let cache_creation = usage
                    .get("cache_creation_input_tokens")
                    .and_then(|v| v.as_u64());
                events.push(AcpEvent::UsageUpdate {
                    session_id: sid(),
                    usage: TurnUsage {
                        model: a.message.model.clone(),
                        input_tokens: input,
                        output_tokens: output,
                        cache_read_input_tokens: cache_read,
                        cache_creation_input_tokens: cache_creation,
                    },
                });
            }
            events
        }
        Message::User(u) => {
            let Ok(v) = serde_json::to_value(u) else {
                return vec![];
            };
            let mut out = vec![];
            if let Some(blocks) = v
                .pointer("/message/content")
                .or_else(|| v.pointer("/content"))
                .and_then(|c| c.as_array())
            {
                for b in blocks {
                    if b.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                        continue;
                    }
                    let id = b.get("tool_use_id").and_then(|t| t.as_str()).unwrap_or("");
                    let tool_name = tool_names
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| "unknown".into());
                    let result = match b.get("content") {
                        Some(serde_json::Value::String(s)) => s.clone(),
                        Some(other) => other.to_string(),
                        None => String::new(),
                    };
                    out.push(AcpEvent::ToolEnd {
                        session_id: sid(),
                        tool_name,
                        result,
                    });
                }
            }
            out
        }
        Message::Result(r) => {
            if r.is_error {
                // sebas-9pz ⑤: a *refusal* (agent declining the request) is
                // NOT a session death — the process is healthy and the next
                // prompt works. Mark it non-terminal so the router keeps the
                // session mapping (card shows ❌ + the refusal text) instead
                // of tearing the session down. Everything else that errors
                // (subtype error_during_execution / error_during_request /
                // ... ) keeps the honest terminal:true — post-error CLI state
                // is unknown.
                let text = r
                    .result
                    .clone()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("claude turn failed ({})", r.subtype));
                let refused = r.subtype.to_lowercase().contains("refusal")
                    || text.to_lowercase().contains("refusal")
                    || text.to_lowercase().contains("refused");
                if refused {
                    vec![AcpEvent::Error {
                        session_id: sid(),
                        message: text,
                        terminal: false,
                    }]
                } else {
                    vec![AcpEvent::Error {
                        session_id: sid(),
                        message: text,
                        terminal: true,
                    }]
                }
            } else {
                let mut events = vec![AcpEvent::Finished { session_id: sid() }];
                if let Some(usage) = &r.usage {
                    let input = usage.get("input_tokens").and_then(|v| v.as_u64());
                    let output = usage.get("output_tokens").and_then(|v| v.as_u64());
                    events.push(AcpEvent::UsageUpdate {
                        session_id: sid(),
                        usage: TurnUsage {
                            model: None,
                            input_tokens: input,
                            output_tokens: output,
                            cache_read_input_tokens: None,
                            cache_creation_input_tokens: None,
                        },
                    });
                }
                events
            }
        }
        // System messages carry model info on session_start; drop others.
        Message::System(s) => {
            if s.subtype == "session_start" {
                if let Some(model) = &s.model {
                    vec![AcpEvent::UsageUpdate {
                        session_id: sid(),
                        usage: TurnUsage {
                            model: Some(model.clone()),
                            input_tokens: None,
                            output_tokens: None,
                            cache_read_input_tokens: None,
                            cache_creation_input_tokens: None,
                        },
                    }]
                } else {
                    vec![]
                }
            } else {
                vec![]
            }
        }
        // StreamEvent (partials disabled), ControlCancelRequest — nothing.
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_to_extra_args_pairs_flags_and_values() {
        let m = args_to_extra_args(&["--model".into(), "sonnet".into(), "--verbose".into()]);
        assert_eq!(m.get("model"), Some(&Some("sonnet".to_string())));
        assert_eq!(m.get("verbose"), Some(&None));
    }

    #[test]
    fn args_to_extra_args_drops_unpairable_positionals() {
        // "stray" follows a complete key/value pair, so it cannot be
        // interpreted as a flag value and is dropped.
        let m = args_to_extra_args(&["--model".into(), "x".into(), "stray".into()]);
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("model"), Some(&Some("x".to_string())));
        // A bare token right after a flag IS treated as that flag's value —
        // indistinguishable from an intended value at this layer.
        let m = args_to_extra_args(&["--verbose".into(), "stray".into()]);
        assert_eq!(m.get("verbose"), Some(&Some("stray".to_string())));
    }

    fn assistant_msg(blocks: serde_json::Value) -> Message {
        let v = serde_json::json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": blocks}
        });
        serde_json::from_value(v).expect("assistant message parses")
    }

    fn assistant_msg_with_usage(
        blocks: serde_json::Value,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Message {
        let v = serde_json::json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": blocks,
                "model": model,
                "usage": {
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens
                }
            }
        });
        serde_json::from_value(v).expect("assistant msg with usage parses")
    }

    #[test]
    fn assistant_text_maps_to_text_delta() {
        let mut names = HashMap::new();
        let m = assistant_msg(serde_json::json!([{"type": "text", "text": "hi"}]));
        let evts = map_message("s1", &mut names, &m);
        assert!(matches!(
            &evts[..],
            [AcpEvent::TextDelta { delta, .. }] if delta == "hi"
        ));
    }

    #[test]
    fn tool_use_records_name_for_later_tool_end() {
        let mut names = HashMap::new();
        let m = assistant_msg(serde_json::json!([
            {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "ls"}}
        ]));
        let evts = map_message("s1", &mut names, &m);
        assert!(matches!(
            &evts[..],
            [AcpEvent::ToolStart { tool_name, args, .. }]
                if tool_name == "Bash" && args == &serde_json::json!({"command": "ls"})
        ));
        assert_eq!(names.get("toolu_1"), Some(&"Bash".to_string()));
    }

    #[test]
    fn user_tool_result_maps_to_tool_end_with_recorded_name() {
        let mut names = HashMap::new();
        names.insert("toolu_1".to_string(), "Bash".to_string());
        let v = serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_1", "content": "ok\n", "is_error": false}
            ]}
        });
        let m: Message = serde_json::from_value(v).expect("user message parses");
        let evts = map_message("s1", &mut names, &m);
        assert!(matches!(
            &evts[..],
            [AcpEvent::ToolEnd { tool_name, result, .. }]
                if tool_name == "Bash" && result == "ok\n"
        ));
    }

    #[test]
    fn user_tool_result_without_recorded_name_is_unknown() {
        let mut names = HashMap::new();
        let v = serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_x", "content": "ok", "is_error": false}
            ]}
        });
        let m: Message = serde_json::from_value(v).expect("user message parses");
        let evts = map_message("s1", &mut names, &m);
        assert!(matches!(
            &evts[..],
            [AcpEvent::ToolEnd { tool_name, .. }] if tool_name == "unknown"
        ));
    }

    #[test]
    fn result_success_maps_to_finished() {
        let mut names = HashMap::new();
        let v = serde_json::json!({
            "type": "result", "subtype": "success", "is_error": false,
            "duration_ms": 1, "duration_api_ms": 1, "num_turns": 1, "session_id": "s1"
        });
        let m: Message = serde_json::from_value(v).expect("result parses");
        let evts = map_message("s1", &mut names, &m);
        assert!(matches!(&evts[..], [AcpEvent::Finished { .. }]));
    }

    #[test]
    fn result_error_maps_to_terminal_error() {
        let mut names = HashMap::new();
        let v = serde_json::json!({
            "type": "result", "subtype": "error_during_execution", "is_error": true,
            "duration_ms": 1, "duration_api_ms": 1, "num_turns": 1, "session_id": "s1"
        });
        let m: Message = serde_json::from_value(v).expect("result parses");
        let evts = map_message("s1", &mut names, &m);
        assert!(matches!(
            &evts[..],
            [AcpEvent::Error { terminal: true, message, .. }] if message.contains("error_during_execution")
        ));
    }

    #[test]
    fn result_refusal_subtype_is_non_terminal() {
        // sebas-9pz ⑤: refusal (subtype carries "refusal") must NOT kill the
        // session — the agent declined but the process is healthy.
        let mut names = HashMap::new();
        let v = serde_json::json!({
            "type": "result", "subtype": "refusal", "is_error": true,
            "duration_ms": 1, "duration_api_ms": 1, "num_turns": 1, "session_id": "s1"
        });
        let m: Message = serde_json::from_value(v).expect("result parses");
        let evts = map_message("s1", &mut names, &m);
        assert!(
            matches!(
                &evts[..],
                [AcpEvent::Error {
                    terminal: false,
                    ..
                }]
            ),
            "refusal must be non-terminal, got {evts:?}"
        );
    }

    #[test]
    fn result_refusal_in_result_text_is_non_terminal() {
        // Some CLI builds report the refusal in the result body rather than
        // the subtype; both must be treated the same.
        let mut names = HashMap::new();
        let v = serde_json::json!({
            "type": "result", "subtype": "error_during_execution", "is_error": true,
            "duration_ms": 1, "duration_api_ms": 1, "num_turns": 1, "session_id": "s1",
            "result": "The model returned a refusal to complete the request"
        });
        let m: Message = serde_json::from_value(v).expect("result parses");
        let evts = map_message("s1", &mut names, &m);
        assert!(
            matches!(
                &evts[..],
                [AcpEvent::Error { terminal: false, message, .. }] if message.contains("refusal")
            ),
            "refusal in result text must be non-terminal, got {evts:?}"
        );
    }

    #[test]
    fn system_frames_are_dropped() {
        let mut names = HashMap::new();
        let v =
            serde_json::json!({"type": "system", "subtype": "thinking_tokens", "session_id": "s1"});
        let m: Message = serde_json::from_value(v).expect("system parses");
        assert!(map_message("s1", &mut names, &m).is_empty());
    }

    #[test]
    fn assistant_with_usage_emits_usage_update() {
        let mut names = HashMap::new();
        let m = assistant_msg_with_usage(
            serde_json::json!([{"type": "text", "text": "hello"}]),
            "claude-sonnet-4-20250514",
            123,
            456,
        );
        let evts = map_message("s1", &mut names, &m);
        // Expect TextDelta + UsageUpdate
        assert_eq!(evts.len(), 2);
        assert!(matches!(&evts[0], AcpEvent::TextDelta { delta, .. } if delta == "hello"));
        match &evts[1] {
            AcpEvent::UsageUpdate { session_id, usage } => {
                assert_eq!(session_id, "s1");
                assert_eq!(usage.model.as_deref(), Some("claude-sonnet-4-20250514"));
                assert_eq!(usage.input_tokens, Some(123));
                assert_eq!(usage.output_tokens, Some(456));
            }
            _ => panic!("expected UsageUpdate"),
        }
    }

    #[test]
    fn assistant_without_usage_does_not_emit_usage_update() {
        let mut names = HashMap::new();
        let m = assistant_msg(serde_json::json!([{"type": "text", "text": "hi"}]));
        let evts = map_message("s1", &mut names, &m);
        assert!(matches!(&evts[..], [AcpEvent::TextDelta { .. }]));
    }

    #[test]
    fn result_success_with_usage_emits_usage_update() {
        let mut names = HashMap::new();
        let v = serde_json::json!({
            "type": "result", "subtype": "success", "is_error": false,
            "duration_ms": 100, "duration_api_ms": 80, "num_turns": 1, "session_id": "s1",
            "usage": {"input_tokens": 200, "output_tokens": 300}
        });
        let m: Message = serde_json::from_value(v).expect("result parses");
        let evts = map_message("s1", &mut names, &m);
        assert_eq!(evts.len(), 2);
        assert!(matches!(&evts[0], AcpEvent::Finished { .. }));
        match &evts[1] {
            AcpEvent::UsageUpdate { session_id, usage } => {
                assert_eq!(session_id, "s1");
                assert_eq!(usage.input_tokens, Some(200));
                assert_eq!(usage.output_tokens, Some(300));
                assert!(usage.model.is_none());
            }
            _ => panic!("expected UsageUpdate"),
        }
    }

    #[test]
    fn system_session_start_emits_model_usage_update() {
        let mut names = HashMap::new();
        let v = serde_json::json!({
            "type": "system", "subtype": "session_start",
            "session_id": "s1", "model": "claude-opus-4-20250514"
        });
        let m: Message = serde_json::from_value(v).expect("system parses");
        let evts = map_message("s1", &mut names, &m);
        assert_eq!(evts.len(), 1);
        match &evts[0] {
            AcpEvent::UsageUpdate { session_id, usage } => {
                assert_eq!(session_id, "s1");
                assert_eq!(usage.model.as_deref(), Some("claude-opus-4-20250514"));
                assert!(usage.input_tokens.is_none());
                assert!(usage.output_tokens.is_none());
            }
            _ => panic!("expected UsageUpdate"),
        }
    }

    #[test]
    fn system_session_start_without_model_emits_nothing() {
        let mut names = HashMap::new();
        let v = serde_json::json!({
            "type": "system", "subtype": "session_start",
            "session_id": "s1"
        });
        let m: Message = serde_json::from_value(v).expect("system parses");
        let evts = map_message("s1", &mut names, &m);
        assert!(evts.is_empty());
    }
}
