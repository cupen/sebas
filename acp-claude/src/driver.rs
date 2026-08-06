//! Engine adapter: drives a real `claude` CLI via `cc-agent-sdk`
//! (stream-json + control protocol) and surfaces the unchanged
//! `AcpEvent`/`AcpCommand` vocabulary the router consumes.
//!
//! Post-ACP design (docs/superpowers/specs/2026-08-06-claude-direct-sdk-refactor-design.md):
//! - One `ClaudeClient` per sebas session; the sebas routing id IS the claude
//!   conversation id (we mint a uuid and pass `--session-id`), so resume is
//!   just `options.resume = Some(same_id)`.
//! - Permissions ride the PreToolUse hook callback (process-internal,
//!   control-request correlated — no socket/hook-script/positional pairing).
//! - `/cancel` = `interrupt()` + respawn-with-resume: the CLI is unusable
//!   after an interrupt (spike §S6), so we transparently reconnect with
//!   `resume` to keep D4 semantics ("cancel the turn, keep the session").
//! - `setting_sources = Some(vec![])` hermetically isolates the child from
//!   the host user's settings/hooks (spike §8b).

use crate::session::{AcpCommand, AcpEvent, Decision, ResponderSlot};
use claude_agent_sdk::{
    ClaudeAgentOptions, ClaudeClient, ContentBlock, HookCallback, HookEvent, HookInput,
    HookJsonOutput, HookMatcher, HookSpecificOutput, Message, PreToolUseHookSpecificOutput,
    SyncHookJsonOutput,
};
use futures::{FutureExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex};

/// Everything needed to establish one claude-backed session.
pub struct ConnectConfig {
    pub claude_path: String,
    pub claude_args: Vec<String>,
    pub work_dir: Option<String>,
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
    evt_tx: mpsc::Sender<AcpEvent>,
    pending_perms: Arc<Mutex<HashMap<String, ResponderSlot>>>,
    /// tool_use_id → tool_name, so User(tool_result) frames can emit ToolEnd
    /// with the tool name (the frames themselves only carry the id).
    tool_names: HashMap<String, String>,
    terminal_sent: Arc<std::sync::atomic::AtomicBool>,
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
    pub async fn connect(cfg: ConnectConfig) -> anyhow::Result<Self> {
        let ConnectConfig {
            claude_path,
            claude_args,
            work_dir,
            session_id,
            resume,
            startup_timeout,
            evt_tx,
            pending_perms,
            terminal_sent,
        } = cfg;

        let mut extra_args = args_to_extra_args(&claude_args);
        extra_args.insert("session-id".into(), Some(session_id.clone()));

        let cb = permission_hook(session_id.clone(), evt_tx.clone(), pending_perms.clone());
        let mut hooks: HashMap<HookEvent, Vec<HookMatcher>> = HashMap::new();
        hooks.insert(
            HookEvent::PreToolUse,
            vec![HookMatcher::builder().hooks(vec![cb]).build()],
        );

        let options = ClaudeAgentOptions {
            cli_path: Some(claude_path.clone().into()),
            cwd: work_dir.clone().map(Into::into),
            hooks: Some(hooks),
            extra_args,
            resume: if resume {
                Some(session_id.clone())
            } else {
                None
            },
            // Hermetic: never load the host user's settings/hooks (spike §8b).
            setting_sources: Some(vec![]),
            ..Default::default()
        };

        let mut client = ClaudeClient::new(options);
        tokio::time::timeout(startup_timeout, client.connect())
            .await
            .map_err(|_| {
                anyhow::anyhow!("acp session start timed out after {:?}", startup_timeout)
            })??;

        Ok(Self {
            client,
            session_id,
            cfg: DriverCfg {
                claude_path,
                claude_args,
                work_dir,
                startup_timeout,
            },
            evt_tx,
            pending_perms,
            tool_names: HashMap::new(),
            terminal_sent,
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
                }
                Sel::Cmd(Some(AcpCommand::Cancel { .. })) => {
                    if !self.handle_cancel().await {
                        return;
                    }
                }
                Sel::Cmd(Some(AcpCommand::CreateSession { prompt, .. }))
                | Sel::Cmd(Some(AcpCommand::ContinueSession { prompt, .. })) => {
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
                    for evt in map_message(&self.session_id, &mut self.tool_names, &m) {
                        let is_terminal = matches!(evt, AcpEvent::Error { terminal: true, .. });
                        if self.evt_tx.send(evt).await.is_err() || is_terminal {
                            return;
                        }
                    }
                }
                Sel::Msg(Some(Err(e))) => {
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
                message: message.into(),
                terminal: true,
            })
            .await;
    }
}

/// Build the PreToolUse hook callback that bridges a claude permission
/// prompt into `AcpEvent::PermissionRequest` and parks the decision oneshot.
/// The manager's `send(PermissionReply)` resolves the oneshot.
fn permission_hook(
    session_id: String,
    evt_tx: mpsc::Sender<AcpEvent>,
    pending: Arc<Mutex<HashMap<String, ResponderSlot>>>,
) -> HookCallback {
    Arc::new(move |input: HookInput, tool_use_id: Option<String>, _ctx| {
        let session_id = session_id.clone();
        let evt_tx = evt_tx.clone();
        let pending = pending.clone();
        async move {
            let HookInput::PreToolUse(pre) = input else {
                return allow_output("non-PreToolUse hook passthrough");
            };
            let request_id = tool_use_id.unwrap_or_else(|| format!("req-{}", uuid::Uuid::new_v4()));
            let (tx, rx) = oneshot::channel();
            pending.lock().await.insert(request_id.clone(), tx);
            // Fire-and-forget: if the router is gone, deny (fail closed).
            let _ = evt_tx
                .send(AcpEvent::PermissionRequest {
                    session_id,
                    request_id,
                    tool_name: pre.tool_name,
                    args: pre.tool_input,
                })
                .await;
            match rx.await {
                Ok(Decision::AllowOnce) | Ok(Decision::AllowSession) => {
                    allow_output("allowed by sebas user")
                }
                Ok(Decision::Deny) | Err(_) => deny_output("denied by sebas user"),
            }
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
        Message::Assistant(a) => a
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
            .collect(),
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
                vec![AcpEvent::Error {
                    session_id: sid(),
                    message: r
                        .result
                        .clone()
                        .unwrap_or_else(|| format!("claude turn failed ({})", r.subtype)),
                    terminal: true,
                }]
            } else {
                vec![AcpEvent::Finished { session_id: sid() }]
            }
        }
        // System (init / hook_started / thinking_tokens / ...), StreamEvent
        // (partials disabled), ControlCancelRequest — nothing to surface.
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
    fn system_frames_are_dropped() {
        let mut names = HashMap::new();
        let v =
            serde_json::json!({"type": "system", "subtype": "thinking_tokens", "session_id": "s1"});
        let m: Message = serde_json::from_value(v).expect("system parses");
        assert!(map_message("s1", &mut names, &m).is_empty());
    }
}
