//! Core session channel client (openspec/changes/add-core-session-channel,
//! tasks 6.1–6.3): a `SessionBackend` implementation over the local-IPC
//! protocol served by the core (Unix socket / Windows named pipe).
//!
//! - Every method opens a short-lived connection: handshake line → ack →
//!   request line → response line.
//! - `subscribe` runs a dedicated streaming connection in a background task
//!   that reconnects with backoff and emits `Resync` after every fresh
//!   snapshot, so views converge without a client restart (6.2).
//! - Unreachable states are reported with their cause (6.3) and — since
//!   cover-core-channel-test-gaps A1.1 — their distinct kind:
//!   `startup failed` (socket absent), `auth rejected` (handshake refused),
//!   `disconnected` (refused connect, post-handshake drop, timeout).

use super::protocol::{
    ChannelHandshake, CoreChannelRequest, CoreChannelResponse, SessionStreamFrame,
};
use super::secret::ChannelSecret;
use async_trait::async_trait;
use sebas_channels::ChannelKey;
use sebas_dispatch::{SessionEvent, SessionInfo, TurnEntry};
use sebas_webui::session_backend::{
    PermissionDecision, PermissionNotice, Reachability, SessionBackend, SessionRejection,
};
use sebas_ipc::{ReadHalf, WriteHalf};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

/// Per-request timeout: a hung core must not wedge the WebUI forever.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Which of the three unreachable states a failed connection attempt produced
/// (A1.1, design D1). The kind is latched next to the cause so
/// [`CoreChannelBackend::reachability`] can map it onto the `Reachability`
/// variants without re-deriving it from cause strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailKind {
    /// Socket absent — the core never came up (or its startup failed).
    StartupFailed,
    /// Handshake rejected — the secret did not match.
    AuthRejected,
    /// Everything else: refused connect, post-handshake drop, timeout.
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnStatus {
    Connected,
    Failed { kind: FailKind, cause: String },
}

pub struct CoreChannelBackend {
    path: PathBuf,
    /// 握手 secret 的动态来源（harden-core-channel-deployment 2.1，D2）：
    /// env 缓存或 secret 文件发现；每次连接前经 `current()` 解析，core 重启
    /// 换钥后重连天然拿到新钥。
    secret: ChannelSecret,
    events: broadcast::Sender<SessionEvent>,
    /// wire-webui-sebas-agent-e2e: native approval notices relayed by the
    /// channel; consumed by the same review-card feed as the in-process backend.
    notices: broadcast::Sender<PermissionNotice>,
    status: std::sync::Mutex<ConnStatus>,
}

impl CoreChannelBackend {
    /// （extract-im-service 4.1）带附件的 ensure 投递：附件经服务端校验后以
    /// 本地路径标记随文本投递。
    pub async fn ensure_message_with(
        &self,
        key: ChannelKey,
        message: String,
        attachments: Vec<crate::core_channel::protocol::Attachment>,
    ) -> Result<(), SessionRejection> {
        match self
            .request(&CoreChannelRequest::EnsureMessage { key, message, attachments })
            .await?
        {
            CoreChannelResponse::Ok => Ok(()),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    pub fn new(path: PathBuf, secret: String) -> Arc<Self> {
        Self::with_secret(path, ChannelSecret::static_value(secret))
    }

    /// 动态 secret 来源形态（D2）：env 已缓存或每次连接前读 secret 文件。
    /// standalone webui / im 走此构造；`new` 保留给常量 secret 的调用方。
    pub fn with_secret(path: PathBuf, secret: ChannelSecret) -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        let (notices, _) = broadcast::channel(64);
        let backend = Arc::new(Self {
            path,
            secret,
            events,
            notices,
            status: std::sync::Mutex::new(ConnStatus::Failed {
                kind: FailKind::StartupFailed,
                cause: "尚未连接 core".into(),
            }),
        });
        // Subscription forwarder: reconnects with backoff for the lifetime
        // of the process (6.2). Started eagerly so the SSE stream comes up
        // with the dashboard.
        let for_forwarder = backend.clone();
        tokio::spawn(async move { for_forwarder.run_forwarder().await });
        backend
    }

    fn set_status(&self, status: ConnStatus) {
        *self.status.lock().unwrap() = status;
    }

    /// Latch a failed connection attempt at the failure point and turn it
    /// into the caller-facing typed rejection.
    fn fail(&self, kind: FailKind, cause: impl Into<String>) -> SessionRejection {
        let cause = cause.into();
        self.set_status(ConnStatus::Failed { kind, cause: cause.clone() });
        unavailable(cause)
    }

    /// One one-shot request: connect → handshake → ack → request → response.
    /// All failure latching happens inside (or below) `request_inner`; only
    /// the outer timeout latches here (the inner future is gone by then).
    async fn request(
        &self,
        req: &CoreChannelRequest,
    ) -> std::result::Result<CoreChannelResponse, SessionRejection> {
        match tokio::time::timeout(REQUEST_TIMEOUT, self.request_inner(req)).await {
            Ok(r) => {
                if r.is_ok() {
                    self.set_status(ConnStatus::Connected);
                }
                r
            }
            Err(_) => {
                let cause = "请求超时".to_string();
                self.set_status(ConnStatus::Failed {
                    kind: FailKind::Disconnected,
                    cause: cause.clone(),
                });
                Err(SessionRejection::Unavailable { cause })
            }
        }
    }

    async fn request_inner(
        &self,
        req: &CoreChannelRequest,
    ) -> std::result::Result<CoreChannelResponse, SessionRejection> {
        let (mut writer, mut reader) = self.connect().await?;
        self.handshake(&mut writer, &mut reader).await?;

        let json = serde_json::to_string(req)
            .map_err(|e| self.fail(FailKind::Disconnected, format!("serialize failed: {e}")))?;
        writer
            .write_all(json.as_bytes())
            .await
            .map_err(|e| self.fail(FailKind::Disconnected, format!("write failed: {e}")))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|e| self.fail(FailKind::Disconnected, format!("write failed: {e}")))?;
        writer
            .flush()
            .await
            .map_err(|e| self.fail(FailKind::Disconnected, format!("write failed: {e}")))?;

        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| self.fail(FailKind::Disconnected, format!("read failed: {e}")))?;
        if line.trim().is_empty() {
            return Err(self.fail(FailKind::Disconnected, "connection dropped"));
        }
        serde_json::from_str::<CoreChannelResponse>(line.trim()).map_err(|e| {
            self.fail(FailKind::Disconnected, format!("parse response failed: {e}"))
        })
    }

    /// Connect against this backend's socket path, latching the failure kind
    /// on error: ENOENT = the core never came up (StartupFailed), everything
    /// else (refused socket file, I/O) is a runtime disconnect.
    async fn connect(
        &self,
    ) -> std::result::Result<(WriteHalf, BufReader<ReadHalf>), SessionRejection> {
        match connect(&self.path).await {
            Ok(pair) => Ok(pair),
            Err((kind, cause)) => {
                self.set_status(ConnStatus::Failed { kind, cause: cause.clone() });
                Err(unavailable(cause))
            }
        }
    }

    /// Send the handshake and wait for the ack, latching AuthRejected on
    /// failure (EOF or bad ack = the secret was rejected; 5.3 server side
    /// closes). A rejected secret is never retried as-is: the next attempt
    /// re-resolves [`self.secret`] (`ChannelSecret::current`), so a rotated
    /// secret file heals on the following connect.
    async fn handshake(
        &self,
        writer: &mut WriteHalf,
        reader: &mut BufReader<ReadHalf>,
    ) -> std::result::Result<(), SessionRejection> {
        match handshake(writer, reader, &self.secret.current()).await {
            Ok(()) => Ok(()),
            Err(cause) => {
                self.set_status(ConnStatus::Failed {
                    kind: FailKind::AuthRejected,
                    cause: cause.clone(),
                });
                Err(unavailable(cause))
            }
        }
    }

    /// The streaming connection loop (6.2): connect, subscribe, forward
    /// events; on any failure sleep with backoff, retry. Failure latching
    /// happens inside `stream_once` at each failure point (connect/handshake
    /// latch their own kinds); this loop never overwrites them.
    async fn run_forwarder(self: Arc<Self>) {
        let mut backoff = Duration::from_secs(1);
        loop {
            // Failure latching happened inside stream_once at each failure
            // point; the outcome itself only drives the retry cadence.
            let outcome = tokio::time::timeout(
                Duration::from_secs(3600),
                self.stream_once(),
            )
            .await
            .unwrap_or(Err("subscription timed out".into()));
            let _ = outcome;
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(15));
        }
    }

    /// One streaming attempt: returns when the connection drops. Resets the
    /// caller's backoff via the shared flag when a fresh snapshot arrives.
    /// Every failure path latches its (kind, cause) status before returning.
    async fn stream_once(&self) -> std::result::Result<(), String> {
        let (mut writer, mut reader) = connect(&self.path).await.map_err(|(kind, cause)| {
            self.set_status(ConnStatus::Failed { kind, cause: cause.clone() });
            cause
        })?;
        handshake(&mut writer, &mut reader, &self.secret.current())
            .await
            .inspect_err(|cause| {
                self.set_status(ConnStatus::Failed {
                    kind: FailKind::AuthRejected,
                    cause: cause.clone(),
                });
            })?;

        let sub = serde_json::to_string(&CoreChannelRequest::Subscribe).map_err(|e| {
            let cause = format!("serialize failed: {e}");
            self.set_status(ConnStatus::Failed {
                kind: FailKind::Disconnected,
                cause: cause.clone(),
            });
            cause
        })?;
        let write_sub = |e| {
            let cause = format!("subscribe write failed: {e}");
            self.set_status(ConnStatus::Failed {
                kind: FailKind::Disconnected,
                cause: cause.clone(),
            });
            cause
        };
        writer
            .write_all(sub.as_bytes())
            .await
            .map_err(write_sub)?;
        writer.write_all(b"\n").await.map_err(write_sub)?;
        writer.flush().await.map_err(write_sub)?;

        let mut line = String::new();
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|e| {
                    let cause = format!("stream read failed: {e}");
                    self.set_status(ConnStatus::Failed {
                        kind: FailKind::Disconnected,
                        cause: cause.clone(),
                    });
                    cause
                })?;
            if n == 0 {
                let cause = "connection dropped".to_string();
                self.set_status(ConnStatus::Failed {
                    kind: FailKind::Disconnected,
                    cause: cause.clone(),
                });
                return Err(cause);
            }
            let frame: SessionStreamFrame = match serde_json::from_str(line.trim()) {
                Ok(f) => f,
                Err(e) => {
                    let cause = format!("parse frame failed: {e}");
                    self.set_status(ConnStatus::Failed {
                        kind: FailKind::Disconnected,
                        cause: cause.clone(),
                    });
                    return Err(cause);
                }
            };
            match frame {
                SessionStreamFrame::Snapshot { .. } => {
                    // Fresh snapshot from the (re)connect: tell subscribers
                    // to re-render from the backend snapshot. Backoff resets
                    // because the connection is demonstrably healthy.
                    self.set_status(ConnStatus::Connected);
                    let _ = self.events.send(SessionEvent::Resync);
                }
                SessionStreamFrame::Event { event } => {
                    self.set_status(ConnStatus::Connected);
                    let _ = self.events.send(event);
                }
                // wire-webui-sebas-agent-e2e: a gated tool call awaiting a
                // decision. Not buffered — operators who miss the review card
                // rely on the kernel's fail-closed path, not on replay.
                SessionStreamFrame::ApprovalRequested { notice } => {
                    self.set_status(ConnStatus::Connected);
                    let _ = self.notices.send(notice);
                }
            }
        }
    }
}

fn unavailable(cause: String) -> SessionRejection {
    SessionRejection::Unavailable { cause }
}

/// fail-fast-on-startup-errors（core-session-channel spec delta / task 2.4）：
/// core 不可达时，若 `SEBAS_STARTUP_ERROR_FILE` 里有最近一次启动失败的摘要
/// （core 的「最近一次启动尝试失败」闩锁，ready 后自清除），把它并进 cause
/// ——webui 的 degradation banner / `/api/summary.reachability.cause` 由此
/// 显示 "core startup failed: <可读原因>"，而不是一句模糊的 socket absent。
/// core 正常恢复（ready 清除闩锁 + 通道 Connected）后 banner 自然消失。
/// D2（cover-core-channel-test-gaps）：无条件 enrich 沿用不收窄——kind 与
/// cause 正交，三类不可达都保留本富化。
fn enrich_with_startup_summary(cause: &str) -> String {
    match crate::startup_failure::read_env_summary() {
        Some(summary) => format!("core startup failed: {summary}"),
        None => cause.to_string(),
    }
}

/// Connect, mapping error kinds onto their distinct cause + failure kind
/// (6.3; A1.1 adds the kind discrimination):
/// - ENOENT (socket absent) → StartupFailed with the path-naming fallback
///   cause (spec scenario "socket-not-found fallback cause");
/// - everything else (refused socket file, I/O) → Disconnected.
async fn connect(
    path: &Path,
) -> std::result::Result<(WriteHalf, BufReader<ReadHalf>), (FailKind, String)> {
    match sebas_ipc::connect(path).await {
        Ok(stream) => {
            let (r, w) = sebas_ipc::split(stream);
            Ok((w, BufReader::new(r)))
        }
        Err(e) => {
            match e.kind() {
                std::io::ErrorKind::NotFound => Err((
                    FailKind::StartupFailed,
                    format!(
                        "core session channel socket not found at {}",
                        path.display()
                    ),
                )),
                std::io::ErrorKind::ConnectionRefused => {
                    Err((FailKind::Disconnected, "connection refused".into()))
                }
                _ => Err((FailKind::Disconnected, "connect failed".into())),
            }
        }
    }
}

/// Send the handshake line and wait for the ack. EOF or a bad ack after the
/// handshake = the secret was rejected (5.3 server side closes) — the caller
/// latches `FailKind::AuthRejected` around this. Cause wording is the spec's
/// "core rejected channel handshake" (A1.2 scenario).
async fn handshake(
    writer: &mut WriteHalf,
    reader: &mut BufReader<ReadHalf>,
    secret: &str,
) -> std::result::Result<(), String> {
    let hs = serde_json::to_string(&ChannelHandshake {
        secret: secret.to_string(),
    })
    .map_err(|e| format!("serialize failed: {e}"))?;
    writer
        .write_all(hs.as_bytes())
        .await
        .map_err(|e| format!("handshake write failed: {e}"))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|e| format!("handshake write failed: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("handshake write failed: {e}"))?;

    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .map_err(|_| "core rejected channel handshake".to_string())?;
    if line.trim().is_empty() {
        return Err("core rejected channel handshake".into());
    }
    #[derive(serde::Deserialize)]
    struct Ack {
        handshake: String,
    }
    match serde_json::from_str::<Ack>(line.trim()) {
        Ok(ack) if ack.handshake == "ok" => Ok(()),
        _ => Err("core rejected channel handshake".into()),
    }
}

#[async_trait]
impl SessionBackend for CoreChannelBackend {
    async fn snapshot(&self) -> Vec<SessionInfo> {
        match self.request(&CoreChannelRequest::Snapshot).await {
            Ok(CoreChannelResponse::Snapshot { sessions }) => sessions,
            Ok(_) => Vec::new(),
            Err(_) => Vec::new(),
        }
    }

    async fn focused(&self) -> Option<ChannelKey> {
        match self.request(&CoreChannelRequest::Focused).await {
            Ok(CoreChannelResponse::Focused { key }) => key,
            _ => None,
        }
    }

    async fn set_focus(&self, key: Option<ChannelKey>) {
        let _ = self.request(&CoreChannelRequest::SetFocus { key }).await;
    }

    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        // 通道帧的 agent 必填（workbench-agent-wire-fix D2）：无 agent 的
        // 调用方（feishu 默认路径）语义是「配置的默认 kind」，此处无法解析
        // 配置——由服务端按空 agent 拒绝，调用方应改用 spawn_with 显式传。
        self.spawn_with(prompt, project_dir, "", None).await
    }

    async fn spawn_with(
        &self,
        prompt: String,
        project_dir: Option<String>,
        agent: &str,
        model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        match self
            .request(&CoreChannelRequest::Spawn {
                prompt,
                project_dir,
                model,
                agent: agent.to_string(),
            })
            .await?
        {
            CoreChannelResponse::Spawned { key } => Ok(key),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    async fn set_session_model(&self, key: ChannelKey, model_id: String) -> Result<(), SessionRejection> {
        match self
            .request(&CoreChannelRequest::SetSessionModel { key, model_id })
            .await?
        {
            CoreChannelResponse::Ok => Ok(()),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    /// 0-turn placeholder (P2 fix): create the session row over the wire
    /// without an agent child — the trait default would fall back to
    /// `spawn("")`, putting the empty prompt on the wire exactly the bug this
    /// fixes. The agent id rides along（workbench-agent-wire-fix D2：
    /// composer 建 0-turn 会话是常态路径，agent 不上线则用户选的 agent 会被
    /// 静默丢弃）。
    async fn create_placeholder(
        &self,
        project_dir: Option<String>,
        agent: &str,
        model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        match self
            .request(&CoreChannelRequest::CreatePlaceholder {
                project_dir,
                model,
                agent: agent.to_string(),
            })
            .await?
        {
            CoreChannelResponse::Spawned { key } => Ok(key),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    async fn message(&self, key: ChannelKey, message: String) -> Result<(), SessionRejection> {
        match self
            .request(&CoreChannelRequest::Message { key, message, attachments: vec![] })
            .await?
        {
            CoreChannelResponse::Ok => Ok(()),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    async fn close(&self, key: ChannelKey) -> Result<(), SessionRejection> {
        match self.request(&CoreChannelRequest::Close { key }).await? {
            CoreChannelResponse::Ok => Ok(()),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    /// （extract-im-service 2.1）ensure 语义走专线请求：服务端跳过存在性
    /// 预检，未知 key 由核心按入站文本历史语义建会话。
    async fn ensure_message(&self, key: ChannelKey, message: String) -> Result<(), SessionRejection> {
        self.ensure_message_with(key, message, Vec::new()).await
    }


    /// （extract-im-service 2.2）取消会话在飞 turn。
    async fn cancel(&self, key: ChannelKey) -> Result<(), SessionRejection> {
        match self.request(&CoreChannelRequest::Cancel { key }).await? {
            CoreChannelResponse::Ok => Ok(()),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    async fn turns(&self, key: ChannelKey, from: u64) -> Result<Vec<TurnEntry>, SessionRejection> {
        match self.request(&CoreChannelRequest::Turns { key, from }).await? {
            CoreChannelResponse::Turns { entries } => Ok(entries),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    async fn reachability(&self) -> Reachability {
        match &*self.status.lock().unwrap() {
            ConnStatus::Connected => Reachability::Reachable,
            ConnStatus::Failed { kind, cause } => {
                // D2: the unconditional fail-fast enrich stays on every
                // unreachable branch; the kind rides alongside it.
                let cause = enrich_with_startup_summary(cause);
                match kind {
                    FailKind::StartupFailed => Reachability::StartupFailed { cause },
                    FailKind::AuthRejected => Reachability::AuthRejected { cause },
                    FailKind::Disconnected => Reachability::Disconnected { cause },
                }
            }
        }
    }

    async fn state_snapshot(&self, domain: &str) -> Option<serde_json::Value> {
        match self
            .request(&CoreChannelRequest::StateSnapshot {
                domain: domain.to_string(),
            })
            .await
        {
            Ok(CoreChannelResponse::StateSnapshot { payload, .. }) => Some(payload),
            _ => None,
        }
    }

    async fn state_mutate(&self, domain: &str, payload: serde_json::Value) -> Result<(), String> {
        match self
            .request(&CoreChannelRequest::StateMutation {
                domain: domain.to_string(),
                payload,
            })
            .await
        {
            Ok(CoreChannelResponse::StateMutationOk) => Ok(()),
            Ok(CoreChannelResponse::Rejected { rejection }) => {
                Err(format!("state mutation rejected: {rejection}"))
            }
            _ => Err("state store 不可用".into()),
        }
    }

    fn permission_requests(&self) -> Option<broadcast::Receiver<PermissionNotice>> {
        Some(self.notices.subscribe())
    }

    async fn answer_permission(&self, request_id: &str, decision: PermissionDecision) -> bool {
        matches!(
            self.request(&CoreChannelRequest::ApprovalAnswer {
                request_id: request_id.to_string(),
                decision,
            })
            .await,
            Ok(CoreChannelResponse::Ok)
        )
    }
}
