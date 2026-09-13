//! Engine adapter: drives a real `claude` CLI via `cc-agent-sdk`
//! (stream-json + control protocol) and surfaces the unchanged
//! `AcpEvent`/`AcpCommand` vocabulary the router consumes.
//!
//! Post-ACP design (see openspec/specs/acp-driver/spec.md; rationale in
//! docs/design-history.md ADR-1):
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

use crate::claude::session::{AcpCommand, AcpEvent, Decision, ResponderSlot, TurnUsage};
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

/// （permission-mode-auto-gate 1.1）驱动主循环与 PreToolUse hook 回调共享的
/// 「当前生效 permission mode」单元。`PermissionMode: Copy`，`std::sync::Mutex`
/// 足够（临界区只做一次拷贝，绝不跨 await 持锁）；与 stderr_tail 同款惯例。
type SharedPermissionMode = Arc<std::sync::Mutex<claude_agent_sdk::PermissionMode>>;

/// 读取共享 mode 单元的当前值（poisoning 不致命——沿用 stderr_tail 的
/// `unwrap_or_else` 惯例）。
fn load_mode(cell: &SharedPermissionMode) -> claude_agent_sdk::PermissionMode {
    *cell.lock().unwrap_or_else(|p| p.into_inner())
}

/// bypass 档判定：控制面 `allow`/`auto` 都映射为 CLI `bypassPermissions`，
/// hook 门控据此直接放行（ask=Default / edit=AcceptEdits 维持请求流）。
fn is_bypass_tier(mode: claude_agent_sdk::PermissionMode) -> bool {
    matches!(mode, claude_agent_sdk::PermissionMode::BypassPermissions)
}

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
    /// （add-agent-mode-selection）spawn 后生效的权限模式（SDK 把它渲染成
    /// 子进程 argv 的 `--permission-mode`）。`None` = 从 `claude_args` 里的
    /// `--permission-mode` 读取（dispatch 放在那里的启动值），两边都没有
    /// 就是 CLI 默认。post-cancel respawn 传当前值，运行时切换过的模式
    /// 在重生后不回退。
    pub permission_mode: Option<claude_agent_sdk::PermissionMode>,
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
    /// (≈SIGKILL). Tied to openspec/specs/acp-driver/spec.md "5min 无任何 notification".
    last_activity: tokio::time::Instant,
    /// Escalation stage: 0..=3 interrupts already sent for the current hang.
    hang_stage: u8,
    /// True while a PreToolUse permission prompt is parked awaiting the
    /// user's click (openspec/specs/acp-driver/spec.md: permission wait is "永不超时"). Hang
    /// detection is suspended while this is set, otherwise a slow user
    /// click would look exactly like a hung child.
    waiting_permission: Arc<std::sync::atomic::AtomicBool>,
    /// True while a turn is in progress (between CreateSession/ContinueSession
    /// and the matching Message::Result). Hang detection only fires when
    /// a turn is active — otherwise the child is idle (waiting for the next
    /// prompt) and must not be killed.
    turn_active: bool,
    /// （add-agent-mode-selection）当前生效的权限模式：spawn 时来自
    /// `--permission-mode` argv（或 ConnectConfig 覆盖），运行时切换成功后
    /// 更新。存活探针发这个值（不能发硬编码 Default——那会把操作者设置的
    /// 模式每秒覆盖回默认）。
    /// （permission-mode-auto-gate 1.1）Arc 共享单元：PreToolUse hook 回调
    /// 每次咨询最前置读取同一份值——bypass 档直接放行，SetMode 更新后下一
    /// 次咨询即时生效（无需 respawn）。
    permission_mode: SharedPermissionMode,
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
            permission_mode,
            startup_timeout,
            evt_tx,
            pending_perms,
            terminal_sent,
        } = cfg;

        // （add-agent-mode-selection）权限模式的单一出处：显式覆盖（post-cancel
        // respawn 携带的运行时值）> argv 里的 `--permission-mode`（dispatch 的
        // 启动值）> CLI 默认。显式覆盖时先从 argv 里摘掉旧 flag，避免双重
        // `--permission-mode`；再以 extra_args 形式回写（SDK 按字典渲染成
        // `--permission-mode <v>`，真 CLI 与 fake-claude 都认）。
        let requested_mode = permission_mode.or_else(|| parse_permission_mode_arg(&claude_args));
        let mut claude_args = claude_args;
        if permission_mode.is_some() {
            claude_args = strip_permission_mode_arg(&claude_args);
        }
        let mut extra_args = args_to_extra_args(&claude_args);
        if let Some(mode) = requested_mode {
            extra_args.insert(
                "permission-mode".into(),
                Some(permission_mode_flag(mode).to_string()),
            );
        }
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
        // （permission-mode-auto-gate 1.1）mode 共享单元：初值 = spawn 生效值
        // （显式覆盖 > argv `--permission-mode` > CLI 默认）。hook 回调与驱动
        // 主循环（SetMode / 存活探针 / respawn）共同持有同一份 Arc。
        let shared_mode: SharedPermissionMode = Arc::new(std::sync::Mutex::new(
            requested_mode.unwrap_or(claude_agent_sdk::PermissionMode::Default),
        ));
        let cb = permission_hook(
            session_id.clone(),
            evt_tx.clone(),
            pending_perms.clone(),
            waiting_permission.clone(),
            shared_mode.clone(),
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
            permission_mode: shared_mode,
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
                    // 当前权限模式是个无害 no-op，CLI 总是应答；任何失败/超时
                    // ⇒ 子进程没了。（add-agent-mode-selection：探针必须发
                    // 会话**当前**的模式——此前的硬编码 Default 会把操作者
                    // 通过 mode=allow/edit 设置的权限模式每秒覆盖回默认。）
                    let probe = self
                        .client
                        .set_permission_mode(load_mode(&self.permission_mode));
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

                    // Hang detection (sebas-9pz ①, openspec/specs/acp-driver/spec.md): the child is
                    // alive but silent for HANG_TIMEOUT. The SDK exposes no
                    // process handle, so SIGTERM/SIGKILL are approximated with
                    // the SDK's own escalation: interrupt() (cancel, ×3), then
                    // disconnect() (closes stdin ≈ SIGTERM), then a 5s grace
                    // before the driver returns — dropping `self.client`
                    // SIGKILLs the child (SubprocessTransport::drop).
                    // HANG_TIMEOUT defaults to openspec/specs/acp-driver/spec.md's 5min; tests override
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
                    // Permission wait suspends hang detection (openspec/specs/acp-driver/spec.md:
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
                Sel::Cmd(Some(AcpCommand::SetMode { mode, .. })) => {
                    // （add-agent-mode-selection）运行时权限模式切换：控制面
                    // 词汇 → SDK PermissionMode，经 control request 下发。
                    // 接受 → 更新当前模式 + ModeChanged；拒绝/失败 → 非终态
                    // Error（mode 不变、会话存活），与 SetModel 同一非致命
                    // 语义。
                    match control_mode_to_permission_mode(&mode) {
                        Some(target) => match self.client.set_permission_mode(target).await {
                            Ok(()) => {
                                // （permission-mode-auto-gate 1.1）写进与 hook
                                // 回调共享的同一单元：下一次 hook 咨询读到新值，
                                // bypass 档即静默放行（无 respawn）。
                                *self
                                    .permission_mode
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner()) = target;
                                let _ = self
                                    .evt_tx
                                    .send(AcpEvent::ModeChanged {
                                        session_id: self.session_id.clone(),
                                        mode: mode.clone(),
                                    })
                                    .await;
                            }
                            Err(e) => {
                                let _ = self
                                    .evt_tx
                                    .send(AcpEvent::Error {
                                        session_id: self.session_id.clone(),
                                        message: format!(
                                            "set mode {mode:?} 被拒绝或未送达（{e}），模式未变"
                                        ),
                                        terminal: false,
                                    })
                                    .await;
                            }
                        },
                        None => {
                            let _ = self
                                .evt_tx
                                .send(AcpEvent::Error {
                                    session_id: self.session_id.clone(),
                                    message: format!("未知 mode {mode:?}，模式未变"),
                                    terminal: false,
                                })
                                .await;
                        }
                    }
                }
                Sel::Cmd(Some(AcpCommand::SetModel { model_id, .. })) => {
                    // 模型选择是 ACP 原生能力（`session/set_config_option`），
                    // Claude 专用驱动不支持。发非终态 Error 并保活会话（acp-driver
                    // spec "Unsupported agent reports explicit error" + acp-model-selection：
                    // 失败后当前模型不变、会话不被销毁）。
                    let _ = self
                        .evt_tx
                        .send(AcpEvent::Error {
                            session_id: self.session_id.clone(),
                            message: format!(
                                "set model {model_id:?} 需要支持 configOptions 的 ACP 会话；当前驱动（Claude 专用）不支持，模型未变"
                            ),
                            terminal: false,
                        })
                        .await;
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
            // 运行时切换过的权限模式在重生后保持（覆盖 argv 里的启动值）。
            permission_mode: Some(load_mode(&self.permission_mode)),
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
/// （permission-mode-auto-gate 1.1）回调最前置先查会话当前生效 mode
/// （`mode` 共享单元）：bypass 档（allow/auto）直接构造 allow 输出返回——
/// 不产生 PermissionRequest、不泊车、不发卡。本层在 driver 内，飞书与
/// webui 等 surface 共用同一驱动路径，天然跨面一致静默。
fn permission_hook(
    session_id: String,
    evt_tx: mpsc::Sender<AcpEvent>,
    pending: Arc<Mutex<HashMap<String, ResponderSlot>>>,
    waiting_permission: Arc<std::sync::atomic::AtomicBool>,
    mode: SharedPermissionMode,
) -> HookCallback {
    use std::sync::atomic::Ordering;
    Arc::new(move |input: HookInput, tool_use_id: Option<String>, _ctx| {
        let session_id = session_id.clone();
        let evt_tx = evt_tx.clone();
        let pending = pending.clone();
        let waiting = waiting_permission.clone();
        let mode = mode.clone();
        async move {
            let HookInput::PreToolUse(pre) = input else {
                return allow_output("non-PreToolUse hook passthrough");
            };
            // 门控最前置（design D1）：每次咨询先读共享 mode 单元。bypass 档
            // （allow/auto → BypassPermissions）直接放行——零请求、零泊车；
            // SetMode 成功即更新同一单元，下一次咨询即时生效（无需 respawn）。
            // 非 bypass 档（ask/edit）维持既有请求流。
            if is_bypass_tier(load_mode(&mode)) {
                return allow_output("allowed by session permission mode (bypass tier)");
            }
            // request_id 以 `claude:` 前缀命名空间化，与 agent-driver spec「request_id
            // as `<kind-slug>:<raw-id>`」一致，避免与通用 ACP 驱动的同名 raw id 在
            // 共享 perm_cards/待决映射里冲突。
            let raw_id = tool_use_id.unwrap_or_else(|| format!("req-{}", uuid::Uuid::new_v4()));
            let request_id = format!("claude:{raw_id}");
            let (tx, rx) = oneshot::channel();
            pending.lock().await.insert(request_id.clone(), tx);
            // Suspend hang detection while the user decides (openspec/specs/acp-driver/spec.md:
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
/// （add-agent-mode-selection）mode 词汇 → SDK 权限模式。**两套词汇都认**：
/// 控制面（`ask`/`edit`/`allow`/`auto`）与 CLI `--permission-mode` 的参数值
/// （`default`/`acceptEdits`/`plan`/`bypassPermissions`）。约定映射：ask=
/// CLI 默认逐次询问，edit=自动接受编辑，allow/auto=完全放行。`None` =
/// 未知词汇（调用方如实报错，不静默降级）。
pub fn control_mode_to_permission_mode(
    mode: &str,
) -> Option<claude_agent_sdk::PermissionMode> {
    // 匹配对象是**已转小写**的输入，CLI 词汇按小写形态书写。
    match mode.trim().to_ascii_lowercase().as_str() {
        "ask" | "default" => Some(claude_agent_sdk::PermissionMode::Default),
        "edit" | "acceptedits" => Some(claude_agent_sdk::PermissionMode::AcceptEdits),
        "allow" | "auto" | "bypasspermissions" => {
            Some(claude_agent_sdk::PermissionMode::BypassPermissions)
        }
        _ => None,
    }
}

/// （add-agent-mode-selection）控制面 mode 词汇 → CLI `--permission-mode`
/// 参数值的约定映射（ask→不传=CLI 默认、edit→acceptEdits、allow/auto→
/// bypassPermissions）。dispatch 组装 argv 与本模块共用这一个出处。
pub fn control_mode_flag(mode: &str) -> Option<&'static str> {
    match control_mode_to_permission_mode(mode) {
        // ask（= CLI default）与未知词汇：spawn argv 不带 flag——前者是 CLI
        // 的自然默认，后者由调用方如实拒绝，都不需要 flag。运行时切回 ask
        // 走 SetMode → `set_permission_mode(Default)` 显式下发，不经过这里。
        None | Some(claude_agent_sdk::PermissionMode::Default) => None,
        Some(m) => Some(permission_mode_flag(m)),
    }
}

/// SDK 权限模式 → CLI `--permission-mode` 参数值。
fn permission_mode_flag(mode: claude_agent_sdk::PermissionMode) -> &'static str {
    match mode {
        claude_agent_sdk::PermissionMode::Default => "default",
        claude_agent_sdk::PermissionMode::AcceptEdits => "acceptEdits",
        claude_agent_sdk::PermissionMode::Plan => "plan",
        claude_agent_sdk::PermissionMode::BypassPermissions => "bypassPermissions",
    }
}

/// 从 argv 里找 `--permission-mode <v>` 并解析成 SDK 权限模式（没有/不认识
/// → `None` = CLI 默认）。
fn parse_permission_mode_arg(args: &[String]) -> Option<claude_agent_sdk::PermissionMode> {
    let value = args
        .iter()
        .enumerate()
        .find(|(_, a)| a.as_str() == "--permission-mode")
        .and_then(|(i, _)| args.get(i + 1))?;
    control_mode_to_permission_mode(value)
}

/// 摘掉 argv 里的 `--permission-mode [v]` 对（显式模式覆盖接管时用，避免
/// 双 flag）。
fn strip_permission_mode_arg(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--permission-mode" {
            i += 1;
            if matches!(args.get(i), Some(v) if !v.starts_with("--")) {
                i += 1;
            }
            continue;
        }
        out.push(args[i].clone());
        i += 1;
    }
    out
}

fn args_to_extra_args(args: &[String]) -> HashMap<String, Option<String>> {    let mut out = HashMap::new();
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
                    // workbench-turn-queue 回归修复：refusal result 帧就是回合
                    // 边界（本函数上方对任意 Result 帧清 turn_active——回合确
                    // 实结束了），所以 Error 之后必须紧跟 Finished 宣告收尾。
                    // 锚定即此配对：收尾由 Finished 驱动（pump 即时路径的
                    // 既有 FSM WORKING→DONE + 队列 drain），游离的非终端
                    // Error（如 SetModel 被拒，不伴随 Result 帧）则永不收尾
                    // ——事件流本身无法按内容区分二者，只有驱动的回合边界
                    // 能锚定「本回合的失败事件」。
                    vec![
                        AcpEvent::Error {
                            session_id: sid(),
                            message: text,
                            terminal: false,
                        },
                        AcpEvent::Finished { session_id: sid() },
                    ]
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

/// The [`crate::AgentDriver`] implementation for the dedicated Claude Code
/// path. Wraps the low-level [`CcDriver`] engine so `SessionManager` can drive
/// it through the driver-agnostic seam without knowing claude specifics
/// (including the resume-rejection → fresh fallback, which is claude-only).
pub struct ClaudeDriver;

#[async_trait::async_trait]
impl crate::agent_driver::AgentDriver for ClaudeDriver {
    async fn spawn(
        &self,
        cfg: crate::agent_driver::DriverConfig,
    ) -> Result<crate::agent_driver::DriverHandle, crate::agent_driver::DriverError> {
        use crate::agent_driver::{DriverConfig, DriverHandle};

        let DriverConfig {
            kind_slug,
            command,
            work_dir,
            extra_env,
            session_id,
            // Reserved for native-ACP agents; Claude's conversation id equals
            // the routing id, so a load is always addressed by the routing id
            // (the `session_id` field). Never used directly here.
            load_session_id: _,
            resume,
            startup_timeout,
            evt_tx,
            cmd_rx,
            cancel_rx,
            pending_perms,
            terminal_sent,
        } = cfg;

        // command[0] is the claude binary; the rest are argv (map-shaped).
        let mut argv = command.into_iter();
        let claude_path = argv.next().unwrap_or_else(|| "claude".to_string());
        let claude_args: Vec<String> = argv.collect();

        let make_connect = |sid: String, resume: bool| ConnectConfig {
            claude_path: claude_path.clone(),
            claude_args: claude_args.clone(),
            work_dir: work_dir.clone(),
            extra_env: extra_env.clone(),
            session_id: sid,
            resume,
            // 初始模式来自 argv 里的 `--permission-mode`（connect 内解析）；
            // DriverConfig 不承载 mode——dispatch 通过 command argv 传递。
            permission_mode: None,
            startup_timeout,
            evt_tx: evt_tx.clone(),
            pending_perms: pending_perms.clone(),
            terminal_sent: terminal_sent.clone(),
        };

        let (driver, session_id, resumed) =
            match CcDriver::connect(make_connect(session_id.clone(), resume)).await {
                Ok(d) => (d, session_id, resume),
                Err(ConnectError::ResumeRejected) => {
                    // Graceful fallback (sebas-dk8.4): the old conversation is
                    // gone (claude's session files were cleaned). Start fresh
                    // with a NEW id instead of failing the spawn.
                    let fresh = uuid::Uuid::new_v4().to_string();
                    tracing::warn!(
                        old = %session_id,
                        fresh = %fresh,
                        kind = %kind_slug,
                        "agent rejected resume; falling back to a fresh session"
                    );
                    let d = CcDriver::connect(make_connect(fresh.clone(), false))
                        .await
                        .map_err(conn_err)?;
                    (d, fresh, false)
                }
                Err(e) => return Err(conn_err(e)),
            };

        let run: futures::future::BoxFuture<'static, ()> =
            Box::pin(async move { driver.run(cmd_rx, cancel_rx).await });

        Ok(DriverHandle {
            session_id,
            resumed,
            // Claude's conversation id IS the routing id — no separate ACP
            // session id to map; resume is addressed by the routing id itself.
            acp_session_id: None,
            // Claude 暴露 configOptions/模型选择（add-acp-model-selection
            // D4：无模型选项的 agent 不显示模型 UI）；handshake=None 时由
            // manager 直接取该字段。
            model: None,
            handshake: None,
            run,
        })
    }
}

/// Map a low-level `ConnectError` into the driver-agnostic `DriverError`.
fn conn_err(e: ConnectError) -> crate::agent_driver::DriverError {
    match e {
        ConnectError::ResumeRejected => {
            crate::agent_driver::DriverError::ResumeRejected("resume rejected".into())
        }
        ConnectError::Other(e) => crate::agent_driver::DriverError::Other(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- add-agent-mode-selection：控制面 mode → CLI flag 约定映射 ----

    #[test]
    fn control_mode_flag_maps_the_four_vocabulary_values() {
        // ask = CLI 默认（不传 flag），edit = 自动接受编辑，allow/auto = 放行。
        assert_eq!(control_mode_flag("ask"), None);
        assert_eq!(control_mode_flag("edit"), Some("acceptEdits"));
        assert_eq!(control_mode_flag("allow"), Some("bypassPermissions"));
        assert_eq!(control_mode_flag("auto"), Some("bypassPermissions"));
    }

    #[test]
    fn control_mode_flag_rejects_unknown_vocabulary() {
        // 未知值（如 plan）如实 None——调用方拒绝或报错，不静默降级。
        assert_eq!(control_mode_flag("plan"), None);
        assert_eq!(control_mode_flag("yolo"), None);
        assert_eq!(control_mode_flag(""), None);
    }

    #[test]
    fn control_mode_flag_accepts_cli_flag_values_too() {
        // driver 解析 argv 里的 `--permission-mode`（CLI 词汇）与运行时切换
        // （控制面词汇）共用一套映射；default/ask 对 spawn 都意味着"不传"。
        assert_eq!(control_mode_flag("default"), None);
        assert_eq!(control_mode_flag("acceptEdits"), Some("acceptEdits"));
        assert_eq!(
            control_mode_flag("bypassPermissions"),
            Some("bypassPermissions")
        );
    }

    #[test]
    fn parse_and_strip_permission_mode_arg_round_trip() {
        let argv = vec![
            "--model".to_string(),
            "sonnet".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
        ];
        assert_eq!(
            parse_permission_mode_arg(&argv),
            Some(claude_agent_sdk::PermissionMode::BypassPermissions)
        );
        let stripped = strip_permission_mode_arg(&argv);
        assert!(!stripped.contains(&"--permission-mode".to_string()));
        assert_eq!(stripped, vec!["--model".to_string(), "sonnet".to_string()]);
        // 值缺失的孤儿 flag 也能被摘掉（不吞掉后面的参数）。
        let orphan = vec!["--permission-mode".to_string(), "--model".to_string()];
        assert_eq!(parse_permission_mode_arg(&orphan), None);
        assert_eq!(
            strip_permission_mode_arg(&orphan),
            vec!["--model".to_string()]
        );
    }

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

    // ---- permission-mode-auto-gate 1.1：hook 门控（mode 共享单元） ----

    /// 构造 PreToolUse hook 输入（形状与 fake-claude 的 hook_callback 帧
    /// 一致，也即真 CLI 的 PreToolUse 载荷）。
    fn pre_tool_use_input(tool_name: &str) -> HookInput {
        let v = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "session_id": "s1",
            "transcript_path": "/tmp/fake.jsonl",
            "cwd": "/tmp",
            "tool_name": tool_name,
            "tool_input": {"command": "rm -rf /"}
        });
        serde_json::from_value(v).expect("PreToolUse hook input parses")
    }

    fn hook_ctx() -> claude_agent_sdk::HookContext {
        claude_agent_sdk::HookContext { signal: None }
    }

    /// 从 hook 输出里取 permissionDecision（none = 输出形状不符）。
    fn permission_decision(out: &HookJsonOutput) -> Option<&str> {
        match out {
            HookJsonOutput::Sync(sync) => match &sync.hook_specific_output {
                Some(HookSpecificOutput::PreToolUse(p)) => p.permission_decision.as_deref(),
                _ => None,
            },
            _ => None,
        }
    }

    /// 一套 hook 咨询环境：回调 + 事件通道 + 待决映射 + hang 挂起位 +
    /// mode 共享单元——正是 `connect()` 里组装的那几件（client 除外）。
    struct HookRig {
        hook: HookCallback,
        evt_rx: mpsc::Receiver<AcpEvent>,
        pending: Arc<Mutex<HashMap<String, ResponderSlot>>>,
        waiting: Arc<std::sync::atomic::AtomicBool>,
        mode: SharedPermissionMode,
    }

    fn hook_rig(mode: claude_agent_sdk::PermissionMode) -> HookRig {
        let (evt_tx, evt_rx) = mpsc::channel(8);
        let pending: Arc<Mutex<HashMap<String, ResponderSlot>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let waiting = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mode_cell: SharedPermissionMode = Arc::new(std::sync::Mutex::new(mode));
        let hook = permission_hook(
            "s1".into(),
            evt_tx,
            pending.clone(),
            waiting.clone(),
            mode_cell.clone(),
        );
        HookRig {
            hook,
            evt_rx,
            pending,
            waiting,
            mode: mode_cell,
        }
    }

    #[test]
    fn bypass_tier_predicate_matches_only_bypass_permissions() {
        // 控制面 allow/auto 都映射为 BypassPermissions（见
        // control_mode_to_permission_mode）；ask/edit 不得误入 bypass 档。
        assert!(is_bypass_tier(claude_agent_sdk::PermissionMode::BypassPermissions));
        assert!(!is_bypass_tier(claude_agent_sdk::PermissionMode::Default));
        assert!(!is_bypass_tier(claude_agent_sdk::PermissionMode::AcceptEdits));
        assert!(!is_bypass_tier(claude_agent_sdk::PermissionMode::Plan));
    }

    /// bypass 档（allow/auto）：hook 直接构造 allow 返回——零请求、零泊车、
    /// 不触碰 hang 挂起位（不发卡路径完全不经过既有泊车机制）。
    #[tokio::test]
    async fn bypass_tier_hook_allows_without_request_or_parking() {
        let mut rig = hook_rig(claude_agent_sdk::PermissionMode::BypassPermissions);
        let out = (rig.hook)(pre_tool_use_input("Bash"), Some("tu-1".into()), hook_ctx()).await;
        assert_eq!(
            permission_decision(&out),
            Some("allow"),
            "bypass tier must resolve allow directly"
        );
        assert!(
            rig.evt_rx.try_recv().is_err(),
            "bypass tier must not produce a PermissionRequest"
        );
        assert!(rig.pending.lock().await.is_empty(), "nothing parked");
        assert!(
            !rig
                .waiting
                .load(std::sync::atomic::Ordering::SeqCst),
            "permission wait flag must stay untouched on the silent path"
        );
    }

    /// 非 bypass 档（ask/edit）：维持既有请求流——产生 PermissionRequest、
    /// 泊车等决定、挂起 hang 检测，决定到达后按决定输出。
    #[tokio::test]
    async fn ask_and_edit_tiers_still_request_and_park() {
        for tier in [
            claude_agent_sdk::PermissionMode::Default,
            claude_agent_sdk::PermissionMode::AcceptEdits,
        ] {
            let mut rig = hook_rig(tier);
            let hook = rig.hook.clone();
            let task = tokio::spawn(
                async move { hook(pre_tool_use_input("Bash"), Some("tu-2".into()), hook_ctx()).await },
            );
            let evt = tokio::time::timeout(Duration::from_secs(2), rig.evt_rx.recv())
                .await
                .expect("event timeout")
                .expect("event channel open");
            let AcpEvent::PermissionRequest { request_id, tool_name, .. } = evt else {
                panic!("non-bypass tier {tier:?} must produce PermissionRequest, got {evt:?}");
            };
            assert_eq!(tool_name, "Bash");
            assert_eq!(rig.pending.lock().await.len(), 1, "decision must be parked");
            assert!(
                rig.waiting.load(std::sync::atomic::Ordering::SeqCst),
                "hang detection suspended while parked (现状流不变)"
            );
            let responder = rig
                .pending
                .lock()
                .await
                .remove(&request_id)
                .expect("parked responder");
            responder.send(Decision::AllowOnce).expect("resolve decision");
            let out = task.await.expect("hook future joins");
            assert_eq!(permission_decision(&out), Some("allow"));
            assert!(rig.pending.lock().await.is_empty(), "parking consumed");
        }
    }

    /// SetMode 更新共享单元后，下一次咨询即时读到新值——同一 hook 实例
    /// （同一驱动、无 respawn）从「照常弹请求」变为「直接放行、零请求」。
    #[tokio::test]
    async fn set_mode_unit_update_takes_effect_on_next_consult_without_respawn() {
        let mut rig = hook_rig(claude_agent_sdk::PermissionMode::Default);
        // 第一次咨询（ask 档）：照常产生请求——证明门控确实在查共享单元。
        let hook = rig.hook.clone();
        let task = tokio::spawn(
            async move { hook(pre_tool_use_input("Bash"), Some("tu-3".into()), hook_ctx()).await },
        );
        let evt = tokio::time::timeout(Duration::from_secs(2), rig.evt_rx.recv())
            .await
            .expect("event timeout")
            .expect("event channel open");
        let AcpEvent::PermissionRequest { request_id, .. } = evt else {
            panic!("ask tier must produce PermissionRequest, got {evt:?}");
        };
        let responder = rig
            .pending
            .lock()
            .await
            .remove(&request_id)
            .expect("parked responder");
        responder.send(Decision::Deny).expect("resolve decision");
        let out = task.await.expect("hook future joins");
        assert_eq!(permission_decision(&out), Some("deny"));
        // 模拟 run() 的 SetMode 分支：SDK set_permission_mode 被接受后写同一
        // 共享单元（测试与驱动持有同一个 Arc）。
        *rig.mode.lock().unwrap_or_else(|p| p.into_inner()) =
            claude_agent_sdk::PermissionMode::BypassPermissions;
        // 第二次咨询：即时生效——直接放行，零请求、零泊车。
        let out2 = (rig.hook)(pre_tool_use_input("Bash"), Some("tu-4".into()), hook_ctx()).await;
        assert_eq!(
            permission_decision(&out2),
            Some("allow"),
            "next consult after SetMode must see the new mode"
        );
        assert!(
            rig.evt_rx.try_recv().is_err(),
            "no PermissionRequest after the mode switch"
        );
        assert!(rig.pending.lock().await.is_empty());
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
                }, AcpEvent::Finished { .. }]
            ),
            "refusal must be non-terminal AND close the turn with Finished, got {evts:?}"
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
                [AcpEvent::Error { terminal: false, message, .. }, AcpEvent::Finished { .. }] if message.contains("refusal")
            ),
            "refusal in result text must be non-terminal AND close the turn, got {evts:?}"
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
