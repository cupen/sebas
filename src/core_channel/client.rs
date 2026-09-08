//! Core session channel client (openspec/changes/add-core-session-channel,
//! tasks 6.1–6.3): a `SessionBackend` implementation over the local-IPC
//! protocol served by the core (Unix socket / Windows named pipe).
//!
//! - Every method opens a short-lived connection: handshake line → ack →
//!   request line → response line.
//! - `subscribe` runs a dedicated streaming connection in a background task
//!   that reconnects with backoff and emits `Resync` after every fresh
//!   snapshot, so views converge without a client restart (6.2).
//! - Unreachable states are reported with their cause (6.3): `socket absent`,
//!   `connection refused`, `secret rejected`, `connection dropped`.

use super::protocol::{
    ChannelHandshake, CoreChannelRequest, CoreChannelResponse, SessionStreamFrame,
};
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnStatus {
    Connected,
    Failed { cause: String },
}

pub struct CoreChannelBackend {
    path: PathBuf,
    /// Current handshake secret. Fixed when constructed with [`Self::new`]
    /// or when the env pins it; otherwise refreshed from the secret file
    /// before every connection (harden-core-channel-deployment D2).
    secret: std::sync::Mutex<String>,
    /// Secret file for discovery; `None` = fixed-secret mode.
    secret_file: Option<PathBuf>,
    /// True when `SEBAS_CORE_SECRET` was set at construction: the env value
    /// is cached and the file is never read on the hot path.
    env_pinned: bool,
    /// Both-missing warn is emitted once per process (spec: warn, not silent).
    warned: std::sync::atomic::AtomicBool,
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
        Self::spawn_with(path, Some(secret), None)
    }

    /// Discovery mode (harden-core-channel-deployment D2): `SEBAS_CORE_SECRET`
    /// env wins and is cached; otherwise the secret file is read before every
    /// connection, so a core restart that mints a new key heals running
    /// clients without intervention. Both missing → one warn, then empty-
    /// secret attempts (never silent, never a crash).
    pub fn new_with_discovery(path: PathBuf, secret_file: PathBuf) -> Arc<Self> {
        Self::spawn_with(path, None, Some(secret_file))
    }

    fn spawn_with(path: PathBuf, fixed_secret: Option<String>, secret_file: Option<PathBuf>) -> Arc<Self> {
        use std::sync::atomic::AtomicBool;
        // Fixed-secret mode keeps its value verbatim (unit tests, explicit
        // wiring). Discovery mode resolves env → file → empty, caching the
        // env value when it pins the secret.
        let (initial, env_pinned) = match &fixed_secret {
            Some(s) => (s.clone(), false),
            None => {
                let probe = secret_file.as_deref().unwrap_or(Path::new(""));
                (
                    super::secret::resolve_secret(probe),
                    super::secret::env_pins_secret(),
                )
            }
        };
        let backend = Arc::new(Self {
            path,
            secret: std::sync::Mutex::new(initial),
            secret_file,
            env_pinned,
            warned: AtomicBool::new(false),
            events: {
                let (events, _) = broadcast::channel(256);
                events
            },
            notices: {
                let (notices, _) = broadcast::channel(64);
                notices
            },
            status: std::sync::Mutex::new(ConnStatus::Failed {
                cause: "尚未连接 core".into(),
            }),
        });
        if fixed_secret.is_none() && backend.current_secret().is_empty() {
            backend.warn_undiscoverable_once();
        }
        // Subscription forwarder: reconnects with backoff for the lifetime
        // of the process (6.2). Started eagerly so the SSE stream comes up
        // with the dashboard.
        let for_forwarder = backend.clone();
        tokio::spawn(async move { for_forwarder.run_forwarder().await });
        backend
    }

    /// The secret for the next connection: cached env/fixed value, or a
    /// fresh read of the secret file (D2 — a rotated file key is picked up
    /// on reconnect with no notification mechanism).
    fn current_secret(&self) -> String {
        if self.env_pinned {
            return self.secret.lock().unwrap().clone();
        }
        if let Some(ref file) = self.secret_file
            && let Some(fresh) = super::secret::read_secret_file(file)
        {
            *self.secret.lock().unwrap() = fresh.clone();
            return fresh;
        }
        if self.secret_file.is_some() {
            self.warn_undiscoverable_once();
        }
        self.secret.lock().unwrap().clone()
    }

    /// Spec scenario "env 与文件皆缺省时启动告警": warn once per process,
    /// keep trying with an empty secret (no crash, no silence).
    fn warn_undiscoverable_once(&self) {
        use std::sync::atomic::Ordering;
        if self.warned.swap(true, Ordering::SeqCst) {
            return;
        }
        let where_ = self
            .secret_file
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        tracing::warn!(
            "核心通道 secret 未找到（{} 未设置且 secret 文件缺失：{}）：以空 secret 尝试连接 core",
            super::secret::SECRET_ENV,
            where_,
        );
    }

    fn set_status(&self, status: ConnStatus) {
        *self.status.lock().unwrap() = status;
    }

    /// One one-shot request: connect → handshake → ack → request → response.
    async fn request(
        &self,
        req: &CoreChannelRequest,
    ) -> std::result::Result<CoreChannelResponse, SessionRejection> {
        match tokio::time::timeout(REQUEST_TIMEOUT, self.request_inner(req)).await {
            Ok(r) => {
                match &r {
                    Ok(_) => self.set_status(ConnStatus::Connected),
                    Err(SessionRejection::Unavailable { cause }) => {
                        self.set_status(ConnStatus::Failed {
                            cause: cause.clone(),
                        })
                    }
                    Err(_) => {}
                }
                r
            }
            Err(_) => {
                let cause = "请求超时".to_string();
                self.set_status(ConnStatus::Failed { cause: cause.clone() });
                Err(SessionRejection::Unavailable { cause })
            }
        }
    }

    async fn request_inner(
        &self,
        req: &CoreChannelRequest,
    ) -> std::result::Result<CoreChannelResponse, SessionRejection> {
        let (mut writer, mut reader) = self.dial().await?;

        let json = serde_json::to_string(req)
            .map_err(|e| unavailable(format!("serialize failed: {e}")))?;
        writer
            .write_all(json.as_bytes())
            .await
            .map_err(|e| unavailable(format!("write failed: {e}")))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|e| unavailable(format!("write failed: {e}")))?;
        writer
            .flush()
            .await
            .map_err(|e| unavailable(format!("write failed: {e}")))?;

        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| unavailable(format!("read failed: {e}")))?;
        if line.trim().is_empty() {
            self.set_status(ConnStatus::Failed {
                cause: "connection dropped".into(),
            });
            return Err(unavailable("connection dropped".into()));
        }
        serde_json::from_str::<CoreChannelResponse>(line.trim())
            .map_err(|e| unavailable(format!("parse response failed: {e}")))
    }

    /// The streaming connection loop (6.2): connect, subscribe, forward
    /// events; on any failure set the status, sleep with backoff, retry.
    async fn run_forwarder(self: Arc<Self>) {
        let mut backoff = Duration::from_secs(1);
        loop {
            let outcome = tokio::time::timeout(
                Duration::from_secs(3600),
                self.stream_once(),
            )
            .await
            .unwrap_or(Err("subscription timed out".into()));
            match outcome {
                // Clean server close (core shutting down): retry after backoff.
                Ok(()) => {}
                Err(cause) => {
                    self.set_status(ConnStatus::Failed { cause });
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(15));
        }
    }

    /// One streaming attempt: returns when the connection drops. Resets the
    /// caller's backoff via the shared flag when a fresh snapshot arrives.
    async fn stream_once(&self) -> std::result::Result<(), String> {
        let (mut writer, mut reader) = self
            .dial()
            .await
            .map_err(|r| match r {
                SessionRejection::Unavailable { cause } => cause,
                other => format!("{other:?}"),
            })?;

        let sub = serde_json::to_string(&CoreChannelRequest::Subscribe)
            .map_err(|e| format!("serialize failed: {e}"))?;
        writer
            .write_all(sub.as_bytes())
            .await
            .map_err(|e| format!("subscribe write failed: {e}"))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|e| format!("subscribe write failed: {e}"))?;
        writer
            .flush()
            .await
            .map_err(|e| format!("subscribe write failed: {e}"))?;

        let mut line = String::new();
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|e| format!("stream read failed: {e}"))?;
            if n == 0 {
                return Err("connection dropped".into());
            }
            let frame: SessionStreamFrame = serde_json::from_str(line.trim())
                .map_err(|e| format!("parse frame failed: {e}"))?;
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

fn is_secret_rejected(r: &SessionRejection) -> bool {
    matches!(r, SessionRejection::Unavailable { cause } if cause == "secret rejected")
}

impl CoreChannelBackend {
    /// Connect + handshake with the current secret, retrying once on a fresh
    /// connection when a file-discovered secret is rejected: the core may
    /// have rotated the file between our resolve and our handshake (D2).
    async fn dial(
        &self,
    ) -> std::result::Result<
        (
            WriteHalf,
            BufReader<ReadHalf>,
        ),
        SessionRejection,
    > {
        let secret = self.current_secret();
        match self.dial_with(&secret).await {
            Ok(v) => Ok(v),
            Err(e) if !self.env_pinned && is_secret_rejected(&e) => {
                let fresh = self.current_secret();
                if fresh != secret {
                    return self.dial_with(&fresh).await;
                }
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    async fn dial_with(
        &self,
        secret: &str,
    ) -> std::result::Result<
        (
            WriteHalf,
            BufReader<ReadHalf>,
        ),
        SessionRejection,
    > {
        let (mut writer, mut reader) = connect(&self.path).await?;
        handshake(&mut writer, &mut reader, secret).await?;
        Ok((writer, reader))
    }
}

/// fail-fast-on-startup-errors（core-session-channel spec delta / task 2.4）：
/// core 不可达时，若 `SEBAS_STARTUP_ERROR_FILE` 里有最近一次启动失败的摘要
/// （core 的「最近一次启动尝试失败」闩锁，ready 后自清除），把它并进 cause
/// ——webui 的 degradation banner / `/api/summary.reachability.cause` 由此
/// 显示 "core startup failed: <可读原因>"，而不是一句模糊的 socket absent。
/// core 正常恢复（ready 清除闩锁 + 通道 Connected）后 banner 自然消失。
fn enrich_with_startup_summary(cause: &str) -> String {
    match crate::startup_failure::read_env_summary() {
        Some(summary) => format!("core startup failed: {summary}"),
        None => cause.to_string(),
    }
}


/// Connect, mapping error kinds onto their distinct causes (6.3).
async fn connect(
    path: &Path,
) -> std::result::Result<
    (
        WriteHalf,
        BufReader<ReadHalf>,
    ),
    SessionRejection,
> {
    match sebas_ipc::connect(path).await {
        Ok(stream) => {
            let (r, w) = sebas_ipc::split(stream);
            Ok((w, BufReader::new(r)))
        }
        Err(e) => {
            let cause = match e.kind() {
                std::io::ErrorKind::NotFound => "socket absent",
                std::io::ErrorKind::ConnectionRefused => "connection refused",
                _ => "connect failed",
            };
            Err(unavailable(cause.to_string()))
        }
    }
}

/// Send the handshake line and wait for the ack. EOF or a bad ack after the
/// handshake = the secret was rejected (5.3 server side closes).
async fn handshake(
    writer: &mut WriteHalf,
    reader: &mut BufReader<ReadHalf>,
    secret: &str,
) -> std::result::Result<(), SessionRejection> {
    let hs = serde_json::to_string(&ChannelHandshake {
        secret: secret.to_string(),
    })
    .map_err(|e| unavailable(format!("serialize failed: {e}")))?;
    writer
        .write_all(hs.as_bytes())
        .await
        .map_err(|e| unavailable(format!("handshake write failed: {e}")))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|e| unavailable(format!("handshake write failed: {e}")))?;
    writer
        .flush()
        .await
        .map_err(|e| unavailable(format!("handshake write failed: {e}")))?;

    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .map_err(|_| unavailable("secret rejected".into()))?;
    if line.trim().is_empty() {
        return Err(unavailable("secret rejected".into()));
    }
    #[derive(serde::Deserialize)]
    struct Ack {
        handshake: String,
    }
    match serde_json::from_str::<Ack>(line.trim()) {
        Ok(ack) if ack.handshake == "ok" => Ok(()),
        _ => Err(unavailable("secret rejected".into())),
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
        self.spawn_with(prompt, project_dir, None, None).await
    }

    async fn spawn_with(
        &self,
        prompt: String,
        project_dir: Option<String>,
        backend: Option<&str>,
        model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        match self
            .request(&CoreChannelRequest::Spawn {
                prompt,
                project_dir,
                model,
                backend: backend.map(str::to_owned),
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
    /// fixes. The execution-backend hint rides along
    /// （add-composer-agent-binding：composer 建 0-turn 会话是常态路径，
    /// hint 不上线则用户在创建模式选的 agent 会被静默丢弃）。
    async fn create_placeholder(
        &self,
        project_dir: Option<String>,
        backend: Option<String>,
        model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        match self
            .request(&CoreChannelRequest::CreatePlaceholder {
                project_dir,
                model,
                backend,
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
            ConnStatus::Failed { cause } => Reachability::Unreachable {
                cause: enrich_with_startup_summary(cause),
            },
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
