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
    ChannelHandshake, ChannelHandshakeAck, CoreChannelRequest, CoreChannelResponse,
    SessionStreamFrame,
};
use super::secret::ChannelSecret;
use async_trait::async_trait;
use sebas_channels::ChannelKey;
use sebas_dispatch::TurnStreamEvent;
use sebas_dispatch::{PendingApproval, PendingSubmission, SessionEvent, SessionInfo, SessionIdentity, TurnEntry};
use sebas_ipc::{ReadHalf, WriteHalf};
use sebas_webui::session_backend::{
    CloseReport, PermissionDecision, PermissionNotice, Reachability, SessionBackend,
    SessionRejection,
};
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
///
/// `pub(super)`：`core_channel::tests` 驱动 `set_status` 翻转序列时构造。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FailKind {
    /// Socket absent — the core never came up (or its startup failed).
    StartupFailed,
    /// Handshake rejected — the secret did not match.
    AuthRejected,
    /// Everything else: refused connect, post-handshake drop, timeout.
    Disconnected,
}

/// `pub(super)`：同 [`FailKind`]——tests 经 `set_status` 做确定性翻转驱动。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConnStatus {
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
    /// 实时回合内容（workbench-live-conversation-flow 1.2）：订阅流里的
    /// `turn` 帧转播到这里，webui WS 面再转播给浏览器。
    turn_events: broadcast::Sender<TurnStreamEvent>,
    /// （add-core-reachability-ws-push D2）可达性翻转广播：`set_status()`
    /// 收口发布，真翻转才发。广播与读端共享同一 ConnStatus→Reachability
    /// 映射（含 startup summary 富化），不另造第二份。
    reachability_tx: broadcast::Sender<Reachability>,
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
            .request(&CoreChannelRequest::EnsureMessage {
                key,
                message,
                attachments,
            })
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
        let (turn_events, _) = broadcast::channel(256);
        let (reachability_tx, _) = broadcast::channel(16);
        let backend = Arc::new(Self {
            path,
            secret,
            events,
            notices,
            turn_events,
            reachability_tx,
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

    /// 节点链路管理（add-remote-execution-node 2.7）：签发配对 token / 列出节点 /
    /// 吊销节点。
    ///
    /// 走通道而不是直接读写注册表文件：注册表是**单写者**文件，只有 core 进程
    /// 持有那一份内存状态；另一个进程写同一个文件会与它互相覆盖。
    pub async fn node_link(
        &self,
        op: crate::core_channel::protocol::NodeLinkOp,
    ) -> Result<crate::core_channel::protocol::NodeLinkOutcome, SessionRejection> {
        match self.request(&CoreChannelRequest::NodeLink { op }).await? {
            CoreChannelResponse::NodeLink(outcome) => Ok(outcome),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    /// 状态收口（add-core-reachability-ws-push D2）：全部 ConnStatus 翻转
    /// 必经此点。发布前与旧值比较——`Connected→Connected` 类重复写（每次
    /// 请求成功都会 latch 一次）不产生帧；真翻转才广播对应的 `Reachability`
    /// （含 startup summary 富化，与 [`Self::reachability`] 读端同一映射）。
    ///
    /// `pub(super)`：`core_channel::tests` 经它做确定性的翻转序列驱动
    /// （公开 API 的翻转与 forwarder 任务交错，无法钉死帧序）。
    pub(super) fn set_status(&self, status: ConnStatus) {
        let flipped = {
            let mut guard = self.status.lock().unwrap();
            if *guard == status {
                false
            } else {
                *guard = status.clone();
                true
            }
        };
        if flipped {
            let _ = self.reachability_tx.send(status_to_reachability(&status));
        }
    }

    /// Latch a failed connection attempt at the failure point and turn it
    /// into the caller-facing typed rejection.
    fn fail(&self, kind: FailKind, cause: impl Into<String>) -> SessionRejection {
        let cause = cause.into();
        self.set_status(ConnStatus::Failed {
            kind,
            cause: cause.clone(),
        });
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
            self.fail(
                FailKind::Disconnected,
                format!("parse response failed: {e}"),
            )
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
                self.set_status(ConnStatus::Failed {
                    kind,
                    cause: cause.clone(),
                });
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
            let outcome = tokio::time::timeout(Duration::from_secs(3600), self.stream_once())
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
            self.set_status(ConnStatus::Failed {
                kind,
                cause: cause.clone(),
            });
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
        writer.write_all(sub.as_bytes()).await.map_err(write_sub)?;
        writer.write_all(b"\n").await.map_err(write_sub)?;
        writer.flush().await.map_err(write_sub)?;

        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await.map_err(|e| {
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
                // 实时回合内容（workbench-live-conversation-flow 1.2）：纯
                // 增量补充，转播即完事；丢失由快照重取收敛。
                SessionStreamFrame::Turn { event } => {
                    self.set_status(ConnStatus::Connected);
                    let _ = self.turn_events.send(event);
                }
                // fix-webui-streaming-liveness 5.2（D6）：core 侧 turn 流
                // 落后的重同步信号——按既有 Resync 语义转播给订阅者（前端
                // 清游标全量重取），连接保持。
                SessionStreamFrame::Resync => {
                    self.set_status(ConnStatus::Connected);
                    let _ = self.events.send(SessionEvent::Resync);
                }
                // （6.1）对端多了一种本 build 不认识的帧：**忽略这一帧**，
                // 连接保持、后续帧照收（未知值不失败解码）。连接状态照旧
                // 标为健康——收到帧本身就证明链路活着。
                SessionStreamFrame::Unknown => {
                    self.set_status(ConnStatus::Connected);
                }
            }
        }
    }
}

fn unavailable(cause: String) -> SessionRejection {
    SessionRejection::Unavailable { cause }
}

/// （add-core-reachability-ws-push D2）ConnStatus → `Reachability` 的共享
/// 映射：`reachability()` 读端与 `set_status()` 广播端共用，startup summary
/// 富化（无条件、不收窄）只活在这一份里。
fn status_to_reachability(status: &ConnStatus) -> Reachability {
    match status {
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

/// 一次性 state domain 查询（unify-router-process-shape 2.2，design D2）：
/// watchdog 停 router 前查 `router_activity` 用——不需要常驻订阅流，一次
/// 连接 + 握手 + 单请求即走。secret 经 [`ChannelSecret::current`] 现场解析
/// （env → secret 文件，与 [`CoreChannelBackend`] 同一发现链）。
///
/// 返回 `None` = core 不可达（socket 缺失 / 握手拒绝 / 超时 / 应答异常）；
/// 调用方按各自的 fail 语义处置（executor 的语义是放行停止）。
pub async fn snapshot_domain_once(
    path: &Path,
    secret: &ChannelSecret,
    domain: &str,
) -> Option<serde_json::Value> {
    let (mut writer, mut reader) = connect(path).await.ok()?;
    handshake(&mut writer, &mut reader, &secret.current())
        .await
        .ok()?;
    let req = serde_json::to_string(&CoreChannelRequest::StateSnapshot {
        domain: domain.to_string(),
    })
    .ok()?;
    use tokio::io::AsyncWriteExt;
    writer.write_all(req.as_bytes()).await.ok()?;
    writer.write_all(b"\n").await.ok()?;
    writer.flush().await.ok()?;

    let mut line = String::new();
    tokio::time::timeout(REQUEST_TIMEOUT, reader.read_line(&mut line))
        .await
        .ok()?
        .ok()?;
    match serde_json::from_str::<CoreChannelResponse>(line.trim()).ok()? {
        CoreChannelResponse::StateSnapshot { payload, .. } => Some(payload),
        _ => None,
    }
}

/// 带启动窗口的有界重试版 [`snapshot_domain_once`]：watchdog 把 core 与
/// webui/im 同时拉起，子进程首次读状态时常早于 core 完成通道 bind（实测
/// 差距仅几毫秒）。只对「core 不可达」（`None`）重试；拿到应答（含 null
/// payload）即刻返回。重试耗尽仍不可达才交回 `None`，由调用方走各自的
/// 降级告警——告警因此只在 core 真不可达时出现。
pub async fn snapshot_domain_with_retry(
    path: &Path,
    secret: &ChannelSecret,
    domain: &str,
    attempts: usize,
    delay: std::time::Duration,
) -> Option<serde_json::Value> {
    for attempt in 0..attempts {
        if attempt > 0 {
            tokio::time::sleep(delay).await;
        }
        if let Some(payload) = snapshot_domain_once(path, secret, domain).await {
            return Some(payload);
        }
    }
    None
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
        Err(e) => match e.kind() {
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
        },
    }
}

/// Send the handshake line and wait for the ack. EOF or a bad ack after the
/// handshake = the secret was rejected (5.3 server side closes) — the caller
/// latches `FailKind::AuthRejected` around this. Cause wording is the spec's
/// "core rejected channel handshake" (A1.2 scenario).
///
/// （3.4②）新客户端读**旧服务端**的 `{"handshake":"ok"}`：`version` 靠 serde
/// 默认值落为 1，握手照旧成功——版本字段是纯 additive 的。
/// （3.2 镜像）服务端报出本端不支持的在先版本时，错误文本**指名双方版本**。
async fn handshake(
    writer: &mut WriteHalf,
    reader: &mut BufReader<ReadHalf>,
    secret: &str,
) -> std::result::Result<(), String> {
    let hs = ChannelHandshake::new(secret.to_string())
        .to_line()
        .map_err(|e| format!("serialize failed: {e}"))?;
    writer
        .write_all(hs.as_bytes())
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
    match ChannelHandshakeAck::from_line(line.trim()) {
        Ok(ChannelHandshakeAck::Ok { version }) if version <= sebas_ipc::protocol::PROTOCOL_VERSION => {
            Ok(())
        }
        Ok(ack) => Err(format!(
            "core rejected channel handshake: {}",
            ack.cause().unwrap_or_else(|| "unknown handshake response".into())
        )),
        Err(_) => Err("core rejected channel handshake".into()),
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

    /// （add-core-reachability-ws-push D1/D2）翻转广播的接收端：发布点在
    /// [`Self::set_status`] 收口，与读端 [`Self::reachability`] 共享同一映射。
    fn reachability_updates(&self) -> broadcast::Receiver<Reachability> {
        self.reachability_tx.subscribe()
    }

    fn subscribe_turn_events(&self) -> broadcast::Receiver<TurnStreamEvent> {
        self.turn_events.subscribe()
    }

    async fn activate(&self, key: ChannelKey) -> Result<bool, SessionRejection> {
        match self.request(&CoreChannelRequest::Activate { key }).await? {
            CoreChannelResponse::Activated { started } => Ok(started),
            CoreChannelResponse::Ok => Ok(false),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        // 通道帧的 agent 必填（workbench-agent-wire-fix D2）：无 agent 的
        // 调用方（feishu 默认路径）语义是「配置的默认 kind」，此处无法解析
        // 配置——由服务端按空 agent 拒绝，调用方应改用 spawn_with 显式传。
        self.spawn_with(prompt, project_dir, "", None, None, None)
            .await
    }

    /// 节点清单（8.2）：经 core 的节点注册表拿——只有 core 有写者句柄。
    ///
    /// 拉不到就如实报错，让前端显示"节点状态不可得"，而不是把"看不见"说成
    /// "没有节点"。
    async fn nodes(&self) -> Result<Vec<sebas_webui::session_backend::NodeInfo>, String> {
        use crate::core_channel::protocol::{NodeLinkOp, NodeLinkOutcome};
        match self
            .request(&CoreChannelRequest::NodeLink {
                op: NodeLinkOp::ListNodes,
            })
            .await
        {
            Ok(CoreChannelResponse::NodeLink(NodeLinkOutcome::Nodes { nodes })) => {
                // 本机是**隐式节点**：它不进注册表（没有握手、没有凭据），但它确实是
                // 这些项目可能落在的节点之一。补在最前面并标 `local: true`，
                // 否则工作台会以为本机项目"属于一个不存在的节点"。
                let mut out = vec![sebas_webui::session_backend::NodeInfo {
                    id: crate::node_link::LOCAL_NODE_ID.to_string(),
                    status: "online".into(),
                    last_seen_unix: None,
                    created_unix: 0,
                    local: true,
                }];
                out.extend(
                    nodes
                        .into_iter()
                        .map(|n| sebas_webui::session_backend::NodeInfo {
                            id: n.id,
                            status: n.status,
                            last_seen_unix: n.last_seen_unix,
                            created_unix: n.created_unix,
                            local: false,
                        }),
                );
                Ok(out)
            }
            Ok(CoreChannelResponse::NodeLink(NodeLinkOutcome::Disabled { cause })) => Err(cause),
            Ok(CoreChannelResponse::NodeLink(NodeLinkOutcome::Failed { cause })) => Err(cause),
            Ok(other) => Err(format!("列出节点得到非预期应答：{other:?}")),
            Err(rejection) => Err(format!("{rejection:?}")),
        }
    }

    /// 请节点自己判定路径（8.1）。
    async fn check_node_path(
        &self,
        node_id: &str,
        path: &str,
    ) -> Result<sebas_webui::session_backend::PathCheck, String> {
        match self
            .request(&CoreChannelRequest::NodePathCheck {
                node_id: node_id.to_string(),
                path: path.to_string(),
            })
            .await
        {
            Ok(CoreChannelResponse::NodePath {
                exists,
                is_dir,
                within_workspace,
            }) => Ok(sebas_webui::session_backend::PathCheck {
                exists,
                is_dir,
                within_workspace,
            }),
            Ok(CoreChannelResponse::Rejected { rejection }) => Err(format!("{rejection:?}")),
            Ok(other) => Err(format!("校验路径得到非预期应答：{other:?}")),
            Err(rejection) => Err(format!("{rejection:?}")),
        }
    }

    async fn spawn_with(
        &self,
        prompt: String,
        project_dir: Option<String>,
        agent: &str,
        model: Option<String>,
        mode: Option<String>,
        node: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        match self
            .request(&CoreChannelRequest::Spawn {
                prompt,
                project_dir,
                model,
                // （add-agent-mode-selection）创建时请求的权限模式随帧上送。
                mode,
                agent: agent.to_string(),
                // 节点维度由调用方给出：`None`/`local` = 本机（与今日一致），
                // 别的值由 core 经节点链路建立——client 自己不解析节点。
                node,
            })
            .await?
        {
            CoreChannelResponse::Spawned { key } => Ok(key),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    async fn set_session_model(
        &self,
        key: ChannelKey,
        model_id: String,
    ) -> Result<(), SessionRejection> {
        match self
            .request(&CoreChannelRequest::SetSessionModel { key, model_id })
            .await?
        {
            CoreChannelResponse::Ok => Ok(()),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    /// （add-agent-mode-selection）中程切换权限模式。命令送达 = `Ok`；执行体
    /// 接受与否经事件流反馈（`ModeChanged` = 成功，非终态 `Error` = 拒绝、
    /// mode 不变）——与 model 切换同一反馈契约。
    async fn set_session_mode(
        &self,
        key: ChannelKey,
        mode: String,
    ) -> Result<(), SessionRejection> {
        match self
            .request(&CoreChannelRequest::SetSessionMode { key, mode })
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
        mode: Option<String>,
        node: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        match self
            .request(&CoreChannelRequest::CreatePlaceholder {
                project_dir,
                model,
                mode,
                agent: agent.to_string(),
                node,
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
            .request(&CoreChannelRequest::Message {
                key,
                message,
                attachments: vec![],
            })
            .await?
        {
            CoreChannelResponse::Ok => Ok(()),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    async fn close(&self, key: ChannelKey) -> Result<CloseReport, SessionRejection> {
        match self.request(&CoreChannelRequest::Close { key }).await? {
            // workbench-turn-queue 5.2：close 结果携带丢弃的待生效提交数。
            CoreChannelResponse::Closed { discarded_pending } => {
                Ok(CloseReport { discarded_pending })
            }
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    /// 归档恢复（fix-webui-qa-defects 2.2）：core 侧重建 Dormant 映射 +
    /// 转写回放；失败以 typed rejection 透传（调用方保留归档条目）。
    /// （round4 3.1）命名来源（label / prompt_preview）随请求迁回 core。
    async fn restore_session(
        &self,
        key: ChannelKey,
        session_id: Option<String>,
        project_dir: Option<String>,
        transcript: Vec<TurnEntry>,
        identity: SessionIdentity,
        label: Option<String>,
        prompt_preview: Option<String>,
    ) -> Result<(), SessionRejection> {
        match self
            .request(&CoreChannelRequest::RestoreSession {
                key,
                session_id,
                project_dir,
                transcript,
                identity,
                label,
                prompt_preview,
            })
            .await?
        {
            CoreChannelResponse::Ok => Ok(()),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    /// fix-webui-approval-restore-and-session-identity 1.2：待批审批读模型
    /// 经 core channel 直达引擎泊车登记（detached webui 刷新重建审批面）。
    async fn pending_approvals(
        &self,
        key: ChannelKey,
    ) -> Result<Vec<PendingApproval>, SessionRejection> {
        match self
            .request(&CoreChannelRequest::PendingApprovals { key })
            .await?
        {
            CoreChannelResponse::PendingApprovals { requests } => Ok(requests),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    /// fix-webui-approval-restore-and-session-identity 5.1：label 设置经 core
    /// channel 落到映射（detached webui 的 rail 重命名）。
    async fn set_session_label(
        &self,
        key: ChannelKey,
        label: Option<String>,
    ) -> Result<(), SessionRejection> {
        match self
            .request(&CoreChannelRequest::SetSessionLabel { key, label })
            .await?
        {
            CoreChannelResponse::Ok => Ok(()),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    /// （workbench-turn-queue 4.3）detached 后端的队列观察面：全量 pending
    /// 随快照/事件自然到达（SessionInfo.pending），这里提供显式读取。
    async fn pending(&self, key: ChannelKey) -> Result<Vec<PendingSubmission>, SessionRejection> {
        // 沿用快照通道：单会话的 pending 从全量快照里取（队列极小，量级
        // 16 + 手打条数；独立 op 会多一条 wire 词汇，无收益）。
        let target = serde_json::to_string(&key).unwrap_or_default();
        Ok(self
            .snapshot()
            .await
            .into_iter()
            .find(|s| serde_json::to_string(&s.channel_key()).unwrap_or_default() == target)
            .map(|s| s.pending)
            .unwrap_or_default())
    }

    async fn remove_pending(
        &self,
        key: ChannelKey,
        pending_id: u64,
    ) -> Result<Vec<PendingSubmission>, SessionRejection> {
        match self
            .request(&CoreChannelRequest::RemovePending { key, pending_id })
            .await?
        {
            CoreChannelResponse::PendingList { pending } => Ok(pending),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    async fn move_pending(
        &self,
        key: ChannelKey,
        pending_id: u64,
        to_index: usize,
    ) -> Result<Vec<PendingSubmission>, SessionRejection> {
        match self
            .request(&CoreChannelRequest::MovePending {
                key,
                pending_id,
                to_index,
            })
            .await?
        {
            CoreChannelResponse::PendingList { pending } => Ok(pending),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    /// （extract-im-service 2.1）ensure 语义走专线请求：服务端跳过存在性
    /// 预检，未知 key 由核心按入站文本历史语义建会话。
    async fn ensure_message(
        &self,
        key: ChannelKey,
        message: String,
    ) -> Result<(), SessionRejection> {
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
        match self
            .request(&CoreChannelRequest::Turns { key, from })
            .await?
        {
            CoreChannelResponse::Turns { entries } => Ok(entries),
            CoreChannelResponse::Rejected { rejection } => Err(rejection),
            other => Err(unavailable(format!("unexpected response: {other:?}"))),
        }
    }

    async fn reachability(&self) -> Reachability {
        status_to_reachability(&self.status.lock().unwrap())
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

    /// add-fetch-models：providers 域抓取 op 经专线帧到 core；结果 = id 列表，
    /// typed rejection 透传 core 的净化 cause。
    async fn fetch_provider_models(&self, provider: &str) -> Result<Vec<String>, String> {
        match self
            .request(&CoreChannelRequest::FetchModels {
                provider: provider.to_string(),
            })
            .await
        {
            Ok(CoreChannelResponse::Models { models, .. }) => Ok(models),
            Ok(CoreChannelResponse::Rejected { rejection }) => {
                Err(format!("fetch_models: {rejection}"))
            }
            _ => Err("fetch_models: core 不可达".into()),
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

#[cfg(test)]
mod retry_tests {
    use super::*;

    /// 只对「core 不可达」重试：路径不存在时每次尝试都返回 None，
    /// 总耗时 ≥ (attempts-1)×delay 证明真的重试了，而不是首试即弃。
    #[tokio::test]
    async fn retries_unreachable_core_before_giving_up() {
        let attempts = 3;
        let delay = std::time::Duration::from_millis(20);
        let started = std::time::Instant::now();
        let out = snapshot_domain_with_retry(
            std::path::Path::new(r"\\.\pipe\sebas/test/no-such-channel"),
            &ChannelSecret::from_env_or_file(None),
            "settings",
            attempts,
            delay,
        )
        .await;
        assert!(out.is_none(), "unreachable core must yield None");
        assert!(
            started.elapsed() >= delay * (attempts as u32 - 1),
            "must keep retrying across the window, took {:?}",
            started.elapsed()
        );
    }
}
