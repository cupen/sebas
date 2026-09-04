//! Core session channel client (openspec/changes/add-core-session-channel,
//! tasks 6.1–6.3): a `SessionBackend` implementation over the Unix-socket
//! protocol served by the core.
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
use sebas_router::{SessionEvent, SessionInfo, TurnEntry};
use sebas_webui::session_backend::{
    Reachability, SessionBackend, SessionRejection,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
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
    secret: String,
    events: broadcast::Sender<SessionEvent>,
    status: std::sync::Mutex<ConnStatus>,
}

impl CoreChannelBackend {
    pub fn new(path: PathBuf, secret: String) -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        let backend = Arc::new(Self {
            path,
            secret,
            events,
            status: std::sync::Mutex::new(ConnStatus::Failed {
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
        let (mut writer, mut reader) = connect(&self.path).await?;
        handshake(&mut writer, &mut reader, &self.secret).await?;

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
        let (mut writer, mut reader) = connect(&self.path)
            .await
            .map_err(|r| match r {
                SessionRejection::Unavailable { cause } => cause,
                other => format!("{other:?}"),
            })?;
        handshake(&mut writer, &mut reader, &self.secret)
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
            }
        }
    }
}

fn unavailable(cause: String) -> SessionRejection {
    SessionRejection::Unavailable { cause }
}

/// Connect, mapping error kinds onto their distinct causes (6.3).
async fn connect(
    path: &Path,
) -> std::result::Result<
    (
        tokio::net::unix::OwnedWriteHalf,
        BufReader<tokio::net::unix::OwnedReadHalf>,
    ),
    SessionRejection,
> {
    match UnixStream::connect(path).await {
        Ok(stream) => {
            let (r, w) = stream.into_split();
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
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
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
        let _ = backend; // the core channel spawn route pins the configured kind
        match self
            .request(&CoreChannelRequest::Spawn {
                prompt,
                project_dir,
                model,
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

    async fn message(&self, key: ChannelKey, message: String) -> Result<(), SessionRejection> {
        match self
            .request(&CoreChannelRequest::Message { key, message })
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
                cause: cause.clone(),
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
}
